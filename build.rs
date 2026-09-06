fn main() {
    println!("cargo:rerun-if-changed=vendor/gpr");
    println!("cargo:rerun-if-changed=vendor/dcraw.c");

    if (std::env::var_os("CARGO_FEATURE_GPR_FALLBACK").is_some()
        || std::env::var_os("CARGO_FEATURE_DCRAW_FALLBACK").is_some())
        && std::env::var_os("HOST") != std::env::var_os("TARGET")
    {
        panic!("native RAW fallbacks require HOST and TARGET to match");
    }

    if std::env::var_os("CARGO_FEATURE_GPR_FALLBACK").is_some() {
        let gpr_tools = build_gpr_tools().expect("Failed to build gpr_tools");
        println!("cargo:rustc-env=GPR_TOOLS_BIN={}", gpr_tools.display());
    }
    if std::env::var_os("CARGO_FEATURE_DCRAW_FALLBACK").is_some() {
        let dcraw_tool = build_dcraw_tool().expect("Failed to build dcraw fallback tool");
        println!("cargo:rustc-env=DCRAW_BIN={}", dcraw_tool.display());
    }
}

fn build_dcraw_tool() -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
    let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR")?);
    let binary = if cfg!(windows) {
        out_dir.join("jpgfromraw-dcraw.exe")
    } else {
        out_dir.join("jpgfromraw-dcraw")
    };

    let compiler = std::env::var_os("CC").unwrap_or_else(|| {
        if cfg!(windows) {
            "cl".into()
        } else {
            "cc".into()
        }
    });
    let mut command = std::process::Command::new(compiler);

    if cfg!(windows) {
        command
            .arg("/nologo")
            .arg("/O2")
            .arg("/DNODEPS")
            .arg("vendor/dcraw.c")
            .arg(format!("/Fe{}", binary.display()));
    } else {
        command
            .arg("-O2")
            .arg("-DNODEPS")
            .arg("vendor/dcraw.c")
            .arg("-o")
            .arg(&binary)
            .arg("-lm");
    }

    run(&mut command)?;
    if !binary.exists() {
        return Err(format!("dcraw fallback binary not found at {}", binary.display()).into());
    }
    Ok(binary)
}

fn build_gpr_tools() -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
    let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR")?);
    let build_dir = out_dir.join("gpr-build");
    std::fs::create_dir_all(&build_dir)?;

    let build_type =
        if std::env::var_os("PROFILE").as_deref() == Some(std::ffi::OsStr::new("release")) {
            "Release"
        } else {
            "Debug"
        };
    run(std::process::Command::new("cmake")
        .arg("vendor/gpr")
        .arg("-B")
        .arg(&build_dir)
        .arg(format!("-DCMAKE_BUILD_TYPE={build_type}")))?;
    run(std::process::Command::new("cmake")
        .arg("--build")
        .arg(&build_dir)
        .arg("--target")
        .arg("gpr_tools")
        .arg("--config")
        .arg(build_type))?;

    let binary_name = if cfg!(windows) {
        "gpr_tools.exe"
    } else {
        "gpr_tools"
    };
    [
        build_dir.join("source/app/gpr_tools").join(binary_name),
        build_dir
            .join(build_type)
            .join("source/app/gpr_tools")
            .join(binary_name),
    ]
    .into_iter()
    .find(|path| path.exists())
    .ok_or_else(|| format!("gpr_tools binary not found in {}", build_dir.display()).into())
}

fn run(command: &mut std::process::Command) -> Result<(), Box<dyn std::error::Error>> {
    let status = command.status()?;
    if !status.success() {
        return Err(format!("command failed with status {status}: {:?}", command).into());
    }
    Ok(())
}
