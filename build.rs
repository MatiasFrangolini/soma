use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=assets/app.rc");
    println!("cargo:rerun-if-changed=assets/soma.ico");
    println!("cargo:rerun-if-env-changed=WINDRES");
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let version = env!("CARGO_PKG_VERSION");
    let numeric_version = format!("{},0", version.replace('.', ","));
    let resource = out_dir.join("soma.rc");
    let template = fs::read_to_string("assets/app.rc").unwrap();
    fs::write(
        &resource,
        template
            .replace("@VERSION_COMMAS@", &numeric_version)
            .replace("@VERSION@", version),
    )
    .unwrap();
    let output = out_dir.join("soma-resource.o");
    let windres = env::var_os("WINDRES").unwrap_or_else(|| "x86_64-w64-mingw32-windres.exe".into());
    let status = Command::new(windres)
        .args(["-I", "assets", "-i"])
        .arg(&resource)
        .args(["-O", "coff", "-o"])
        .arg(&output)
        .status()
        .expect("Windows resource compiler not found; set WINDRES");
    assert!(status.success(), "Could not compile the Soma icon resource");
    println!("cargo:rustc-link-arg-bin=Soma={}", output.display());
}
