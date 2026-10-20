//! 配列盤の生成の並びを、環境変数 `HIMAWARI_MB_ORDER` で選ぶ（ADR-0241の第16回）。
//!
//! - `core`（既定）: coreの `generate` と同じ並び。探索の木は統制群 `legal-gen` と一致する
//! - `type`: 駒種ごとに回す並び。手の集合は同じで並びが変わるので、探索の木も変わる
//!
//! 並びの変わる版は cfg `mb_order_type` で切り替える。照合のテストは、並びがcoreと同じ構成では
//! 順まで、変わる構成では手の集合で比べる（`ORDER_MATCHES_CORE`）。

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-env-changed=HIMAWARI_MB_ORDER");
    println!("cargo::rustc-check-cfg=cfg(mb_order_type)");
    match std::env::var("HIMAWARI_MB_ORDER").as_deref() {
        Err(_) | Ok("") | Ok("core") => {}
        Ok("type") => println!("cargo::rustc-cfg=mb_order_type"),
        Ok(other) => panic!("HIMAWARI_MB_ORDER は core か type: {other}"),
    }
}
