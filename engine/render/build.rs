//! Compiles every Slang program in `shaders/` to SPIR-V in `OUT_DIR` (and
//! the variants a program lists, see below). Modules
//! they import live in `shaders/lib/`. The compiler is `slangc`, found through
//! `SLANGC`, the Vulkan SDK, or `PATH`.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let shaders = Path::new("shaders");
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    println!("cargo::rerun-if-changed=shaders");
    println!("cargo::rerun-if-env-changed=SLANGC");

    let slangc = find_slangc();
    let mut programs: Vec<PathBuf> = std::fs::read_dir(shaders)
        .expect("shaders/ folder")
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "slang"))
        .collect();
    programs.sort();

    for program in programs {
        let name = program.file_stem().unwrap().to_str().unwrap().to_owned();
        compile(&slangc, &program, &out_dir.join(format!("{name}.spv")), None);
        // A line `// variant: <DEFINE> <suffix>` also builds the program with
        // DEFINE set, as `<name>_<suffix>.spv`.
        let source = std::fs::read_to_string(&program).unwrap();
        for variant in source.lines().filter_map(|line| line.strip_prefix("// variant: ")) {
            let (define, suffix) = variant.split_once(' ').expect("variant: <DEFINE> <suffix>");
            compile(&slangc, &program, &out_dir.join(format!("{name}_{}.spv", suffix.trim())), Some(define));
        }
    }
}

fn compile(slangc: &Path, program: &Path, output: &Path, define: Option<&str>) {
    let mut command = Command::new(slangc);
    command
        .arg(program)
        .args(["-I", "shaders/lib"])
        .args(["-target", "spirv", "-profile", "spirv_1_5"])
        // Matrices are laid out like glam's, and `mul(m, v)` is `m * v`.
        .arg("-matrix-layout-column-major")
        .arg("-fvk-use-entrypoint-name")
        // Slang widens the profile by itself for the capabilities a shader uses.
        .args(["-warnings-disable", "41012"])
        .arg("-O2");
    if let Some(define) = define {
        command.arg(format!("-D{define}"));
    }
    let result = command.arg("-o").arg(output).output().unwrap_or_else(|err| panic!("cannot run {}: {err}", slangc.display()));
    let messages = String::from_utf8_lossy(&result.stderr);
    if !result.status.success() || !output.exists() {
        panic!("{} failed to compile:
{messages}", program.display());
    }
    let name = output.file_stem().unwrap().to_string_lossy();
    for line in messages.lines().filter(|line| !line.trim().is_empty()) {
        println!("cargo::warning={name}: {line}");
    }
}

fn find_slangc() -> PathBuf {
    let exe = if cfg!(windows) { "slangc.exe" } else { "slangc" };
    if let Some(path) = env::var_os("SLANGC") {
        return PathBuf::from(path);
    }
    if let Some(sdk) = env::var_os("VULKAN_SDK") {
        let path = Path::new(&sdk).join("Bin").join(exe);
        if path.exists() {
            return path;
        }
    }
    // The SDK installs under C:\VulkanSDK\<version>; take the newest one.
    if cfg!(windows)
        && let Ok(entries) = std::fs::read_dir("C:\\VulkanSDK")
    {
        let mut versions: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path().join("Bin").join(exe)).filter(|p| p.exists()).collect();
        versions.sort();
        if let Some(newest) = versions.pop() {
            return newest;
        }
    }
    PathBuf::from(exe)
}
