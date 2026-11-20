use serde::{Deserialize, Serialize};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::{JobObjects::*, ProcessStatus::*, Threading::*},
};

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct TreeUsage {
    pub phase: String,
    pub cpu_ms: Option<u64>,
    pub read_bytes: Option<u64>,
    pub peak_commit_bytes: Option<u64>,
    pub sampled_peak_private_working_set_bytes: Option<u64>,
    pub working_set_samples: u64,
}

fn query<T>(job: HANDLE, class: JOBOBJECTINFOCLASS, out: &mut T) -> bool {
    unsafe {
        QueryInformationJobObject(
            job,
            class,
            out as *mut T as _,
            std::mem::size_of::<T>() as _,
            std::ptr::null_mut(),
        ) != 0
    }
}

#[repr(C)]
struct Pids {
    assigned: u32,
    count: u32,
    ids: [usize; 256],
}

impl TreeUsage {
    pub(crate) fn sample_memory(&mut self, job: HANDLE) {
        let mut pids = Pids {
            assigned: 0,
            count: 0,
            ids: [0; 256],
        };
        if !query(job, JobObjectBasicProcessIdList, &mut pids)
            || pids.count == 0
            || pids.count != pids.assigned
        {
            return;
        }
        let mut private = 0;
        for &pid in &pids.ids[..pids.count as usize] {
            unsafe {
                let process =
                    OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid as u32);
                if process.is_null() {
                    return;
                }
                let mut info = PROCESS_MEMORY_COUNTERS_EX2::default();
                info.cb = std::mem::size_of_val(&info) as u32;
                let mut member = 0;
                let ok = IsProcessInJob(process, job, &mut member) != 0
                    && member != 0
                    && K32GetProcessMemoryInfo(process, &mut info as *mut _ as _, info.cb) != 0;
                CloseHandle(process);
                if !ok {
                    return;
                }
                private += info.PrivateWorkingSetSize as u64;
            }
        }
        self.working_set_samples += 1;
        self.sampled_peak_private_working_set_bytes =
            Some(private.max(self.sampled_peak_private_working_set_bytes.unwrap_or(0)));
    }

    pub(crate) fn accounting(&mut self, job: HANDLE) {
        let mut counters = JOBOBJECT_BASIC_AND_IO_ACCOUNTING_INFORMATION::default();
        if query(job, JobObjectBasicAndIoAccountingInformation, &mut counters) {
            self.cpu_ms = Some(
                ((counters.BasicInfo.TotalKernelTime + counters.BasicInfo.TotalUserTime) / 10_000)
                    as u64,
            );
            self.read_bytes = Some(counters.IoInfo.ReadTransferCount);
        }
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        if query(job, JobObjectExtendedLimitInformation, &mut limits) {
            self.peak_commit_bytes = Some(limits.PeakJobMemoryUsed as u64);
        }
    }
}
