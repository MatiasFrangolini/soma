use crate::{model::Result, wide};
use serde::{Deserialize, Serialize};
use std::{ffi::c_void, net::IpAddr};
use windows_sys::Win32::{
    Foundation::*, NetworkManagement::IpHelper::*, Networking::WinHttp::*, System::JobObjects::*,
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Readiness {
    pub url: String,
    #[serde(default = "timeout")]
    pub timeout_seconds: u64,
    #[serde(default)]
    pub json_field: Option<String>,
    #[serde(default = "expected")]
    pub expected: String,
}
fn timeout() -> u64 {
    120
}
fn expected() -> String {
    "UP".into()
}

pub struct Target {
    host: String,
    address: IpAddr,
    port: u16,
    path: String,
    secure: bool,
}
impl Readiness {
    pub fn target(&self) -> Result<Target> {
        let (secure, raw) = if let Some(s) = self.url.strip_prefix("http://") {
            (false, s)
        } else if let Some(s) = self.url.strip_prefix("https://") {
            (true, s)
        } else {
            return Err("Readiness requiere una URL HTTP/HTTPS local".into());
        };
        let (authority, path) = raw.split_once('/').unwrap_or((raw, ""));
        let (host, port) = if authority.starts_with('[') {
            let end = authority.find(']').ok_or("Host readiness inválido")?;
            (&authority[1..end], authority[end + 1..].strip_prefix(':'))
        } else {
            authority
                .split_once(':')
                .map(|(h, p)| (h, Some(p)))
                .unwrap_or((authority, None))
        };
        let address: IpAddr = if host.eq_ignore_ascii_case("localhost") {
            "127.0.0.1".parse().unwrap()
        } else {
            host.parse()
                .map_err(|_| "Readiness admite localhost o una IP de loopback")?
        };
        if !address.is_loopback() || path.contains('#') || self.url.len() > 2048 {
            return Err("Readiness requiere una dirección local de loopback válida".into());
        }
        let port = port
            .map(str::parse::<u16>)
            .transpose()
            .map_err(|_| "Puerto readiness inválido")?
            .unwrap_or(if secure { 443 } else { 80 });
        if port == 0 || !(1..=3600).contains(&self.timeout_seconds) {
            return Err("Puerto o timeout readiness inválido".into());
        }
        Ok(Target {
            host: address.to_string(),
            address,
            port,
            path: format!("/{path}"),
            secure,
        })
    }
}

struct Http(*mut c_void);
impl Drop for Http {
    fn drop(&mut self) {
        unsafe {
            if !self.0.is_null() {
                WinHttpCloseHandle(self.0);
            }
        }
    }
}

impl Target {
    pub fn probe(&self, spec: &Readiness, job: HANDLE) -> bool {
        // A healthy server already on this port must never make another run ready.
        if !self.owned_listener(job) {
            return false;
        }
        unsafe {
            let session = Http(WinHttpOpen(
                wide("Soma/0.4").as_ptr(),
                WINHTTP_ACCESS_TYPE_NO_PROXY,
                std::ptr::null(),
                std::ptr::null(),
                0,
            ));
            if session.0.is_null() {
                return false;
            }
            WinHttpSetTimeouts(session.0, 750, 750, 750, 750);
            let connection = Http(WinHttpConnect(
                session.0,
                wide(&self.host).as_ptr(),
                self.port,
                0,
            ));
            if connection.0.is_null() {
                return false;
            }
            let request = Http(WinHttpOpenRequest(
                connection.0,
                wide("GET").as_ptr(),
                wide(&self.path).as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                if self.secure { WINHTTP_FLAG_SECURE } else { 0 },
            ));
            if request.0.is_null() {
                return false;
            }
            let disabled = WINHTTP_DISABLE_REDIRECTS;
            WinHttpSetOption(
                request.0,
                WINHTTP_OPTION_DISABLE_FEATURE,
                &disabled as *const _ as _,
                4,
            );
            if WinHttpSendRequest(request.0, std::ptr::null(), 0, std::ptr::null(), 0, 0, 0) == 0
                || WinHttpReceiveResponse(request.0, std::ptr::null_mut()) == 0
            {
                return false;
            }
            let mut status = 0u32;
            let mut length = 4;
            if WinHttpQueryHeaders(
                request.0,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                std::ptr::null(),
                &mut status as *mut _ as _,
                &mut length,
                std::ptr::null_mut(),
            ) == 0
                || !(200..300).contains(&status)
            {
                return false;
            }
            if spec.json_field.is_none() {
                return true;
            }
            let mut body = Vec::new();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            loop {
                if std::time::Instant::now() >= deadline {
                    return false;
                }
                let mut chunk = [0u8; 2048];
                let mut read = 0;
                if WinHttpReadData(
                    request.0,
                    chunk.as_mut_ptr() as _,
                    chunk.len() as u32,
                    &mut read,
                ) == 0
                {
                    return false;
                }
                if read == 0 {
                    break;
                }
                if body.len() + read as usize > 16 * 1024 {
                    return false;
                }
                body.extend_from_slice(&chunk[..read as usize]);
            }
            serde_json::from_slice::<serde_json::Value>(&body)
                .ok()
                .and_then(|value| {
                    value
                        .get(spec.json_field.as_deref().unwrap())?
                        .as_str()
                        .map(str::to_owned)
                })
                .is_some_and(|value| value == spec.expected)
        }
    }

    fn owned_listener(&self, job: HANDLE) -> bool {
        #[repr(C)]
        struct Pids {
            assigned: u32,
            count: u32,
            ids: [usize; 256],
        }
        unsafe {
            let mut pids: Pids = std::mem::zeroed();
            if QueryInformationJobObject(
                job,
                JobObjectBasicProcessIdList,
                &mut pids as *mut _ as _,
                std::mem::size_of::<Pids>() as u32,
                std::ptr::null_mut(),
            ) == 0
            {
                return false;
            }
            let owners = &pids.ids[..(pids.count as usize).min(256)];
            let family = if self.address.is_ipv4() { 2 } else { 23 };
            let mut size = 0;
            GetExtendedTcpTable(
                std::ptr::null_mut(),
                &mut size,
                0,
                family,
                TCP_TABLE_OWNER_PID_LISTENER,
                0,
            );
            if !(4..=1024 * 1024).contains(&size) {
                return false;
            }
            let mut table = vec![0u8; size as usize];
            if GetExtendedTcpTable(
                table.as_mut_ptr() as _,
                &mut size,
                0,
                family,
                TCP_TABLE_OWNER_PID_LISTENER,
                0,
            ) != 0
            {
                return false;
            }
            let count = u32::from_ne_bytes(table[..4].try_into().unwrap()) as usize;
            for index in 0..count {
                let (port, owner, matches) = match self.address {
                    IpAddr::V4(address) => {
                        let offset = 4 + index * std::mem::size_of::<MIB_TCPROW_OWNER_PID>();
                        if offset + std::mem::size_of::<MIB_TCPROW_OWNER_PID>() > table.len() {
                            return false;
                        }
                        let row = std::ptr::read_unaligned(
                            table.as_ptr().add(offset) as *const MIB_TCPROW_OWNER_PID
                        );
                        (
                            row.dwLocalPort,
                            row.dwOwningPid,
                            row.dwLocalAddr == 0
                                || row.dwLocalAddr.to_ne_bytes() == address.octets(),
                        )
                    }
                    IpAddr::V6(address) => {
                        let offset = 4 + index * std::mem::size_of::<MIB_TCP6ROW_OWNER_PID>();
                        if offset + std::mem::size_of::<MIB_TCP6ROW_OWNER_PID>() > table.len() {
                            return false;
                        }
                        let row = std::ptr::read_unaligned(
                            table.as_ptr().add(offset) as *const MIB_TCP6ROW_OWNER_PID
                        );
                        (
                            row.dwLocalPort,
                            row.dwOwningPid,
                            row.ucLocalAddr == [0; 16] || row.ucLocalAddr == address.octets(),
                        )
                    }
                };
                if u16::from_be(port as u16) == self.port
                    && matches
                    && owners.contains(&(owner as usize))
                {
                    return true;
                }
            }
            false
        }
    }
}
