//! Build script: compile the Objective-C Vision OCR helper on macOS

fn main() {
    // Only compile the Vision helper on macOS
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() == "macos" {
        // Link against required frameworks
        println!("cargo:rustc-link-lib=framework=Vision");
        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rustc-link-lib=framework=CoreGraphics");
        println!("cargo:rustc-link-lib=framework=ImageIO");
        println!("cargo:rustc-link-lib=framework=Cocoa");

        // Compile the Objective-C helpers
        cc::Build::new()
            .file("src/vision_helper.m")
            .file("src/window_helper.m")
            .flag("-fobjc-arc")
            .flag("-x")
            .flag("objective-c")
            .compile("rpick_objc_helpers");
    }
}
