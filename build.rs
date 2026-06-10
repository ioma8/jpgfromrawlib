fn main() {
    let gpr_tools = build_gpr_tools().expect("Failed to build gpr_tools");
    let dcraw_tool = build_dcraw_tool().expect("Failed to build dcraw fallback tool");

    println!("cargo:rustc-env=GPR_TOOLS_BIN={}", gpr_tools.display());
    println!("cargo:rustc-env=DCRAW_BIN={}", dcraw_tool.display());
    println!("cargo:rerun-if-changed=vendor/gpr");

    println!("cargo:rerun-if-changed=vendor/dcraw.c");
}

fn build_dcraw_tool() -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
    let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR")?);
    let binary = if cfg!(windows) {
        out_dir.join("jpgfromraw-dcraw.exe")
    } else {
        out_dir.join("jpgfromraw-dcraw")
    };

    let compiler = cc::Build::new().get_compiler();
    let mut command = std::process::Command::new(compiler.path());
    command.args(compiler.args());

    if compiler.is_like_msvc() {
        command
            .arg("/DNODEPS")
            .arg("vendor/dcraw.c")
            .arg(format!("/Fe{}", binary.display()));
    } else {
        command
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

    run(std::process::Command::new("cmake")
        .arg("vendor/gpr")
        .arg("-B")
        .arg(&build_dir))?;
    run(std::process::Command::new("cmake")
        .arg("--build")
        .arg(&build_dir)
        .arg("--target")
        .arg("gpr_tools"))?;

    let binary = if cfg!(windows) {
        build_dir.join("source/app/gpr_tools/gpr_tools.exe")
    } else {
        build_dir.join("source/app/gpr_tools/gpr_tools")
    };
    if !binary.exists() {
        return Err(format!("gpr_tools binary not found at {}", binary.display()).into());
    }
    Ok(binary)
}

fn run(command: &mut std::process::Command) -> Result<(), Box<dyn std::error::Error>> {
    let status = command.status()?;
    if !status.success() {
        return Err(format!("command failed with status {status}: {:?}", command).into());
    }
    Ok(())
}
