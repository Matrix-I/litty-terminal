use std::{env, path::PathBuf, process::Command};

fn run(command: &mut Command) {
    let status = command.status().expect("run native notification build tool");
    assert!(status.success(), "native notification build failed: {status}");
}

fn main() {
    println!("cargo:rerun-if-changed=src/platform/macos/notifications.m");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") { return; }
    let arch = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") => "arm64", Ok("x86_64") => "x86_64", other => panic!("unsupported Apple target: {other:?}"),
    };
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    let object = out.join("notifications.o");
    run(Command::new("xcrun").args(["clang", "-arch", arch, "-fobjc-arc", "-fblocks", "-mmacosx-version-min=11.0", "-Wall", "-Wextra", "-Werror", "-c", "src/platform/macos/notifications.m", "-o"]).arg(&object));
    run(Command::new("xcrun").args(["ar", "crs"]).arg(out.join("liblitty_notifications.a")).arg(object));
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=litty_notifications");
    println!("cargo:rustc-link-lib=framework=UserNotifications");
    println!("cargo:rustc-link-lib=framework=AppKit");
}
