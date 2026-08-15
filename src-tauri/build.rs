fn main() {
    // 测试构建开关：WHALITO_TEST_BUILD=1 → 编译期 cfg(whalito_test)。
    // 生产构建不设置该变量，测试分支被编译器直接剔除（零残留）。
    println!("cargo:rerun-if-env-changed=WHALITO_TEST_BUILD");
    println!("cargo::rustc-check-cfg=cfg(whalito_test)");
    if std::env::var("WHALITO_TEST_BUILD").map_or(false, |v| v == "1") {
        println!("cargo:rustc-cfg=whalito_test");
    }
    tauri_build::build()
}
