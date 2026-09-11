fn main() {
    #[cfg(feature = "tauri-app")]
    {
        tauri_build::build();
        if std::env::var_os("CARGO_CFG_TARGET_OS").as_deref()
            == Some(std::ffi::OsStr::new("windows"))
            && std::env::var_os("CARGO_CFG_TARGET_ENV").as_deref()
                == Some(std::ffi::OsStr::new("msvc"))
        {
            println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
            println!(
                "cargo:rustc-link-arg=/MANIFESTDEPENDENCY:type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'"
            );
            // Tauri already embeds the application manifest in its generated resource.lib.
            // Prevent link.exe from generating a second MANIFEST #1 for binary targets while
            // retaining the activation manifest above for Rust test harness executables.
            println!("cargo:rustc-link-arg-bin=grafyn=/MANIFEST:NO");
            println!("cargo:rustc-link-arg-bin=grafyn-test-runtime=/MANIFEST:NO");
        }
    }
}
