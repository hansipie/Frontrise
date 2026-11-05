fn main() {
    // Make `scene::eglTerminate` visible to shared libraries (GStreamer GL),
    // so it takes precedence over libEGL's. See its documentation.
    println!("cargo:rustc-link-arg-bins=-Wl,--export-dynamic-symbol=eglTerminate");
}
