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
        }
    }
}
