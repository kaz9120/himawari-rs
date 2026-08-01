//! ネットワーク構造の次元をビルド時に決める（ADR-0127）。
//!
//! 環境変数 `HIMAWARI_ARCH` に `<FT>x<L1>x<L2>` を渡すと、その構成で
//! ビルドする。省略すると既定構成になる。
//!
//! featureで持つと構成の数だけfeatureが要り、組み合わせが積で増える。
//! 1つの文字列で受ければ構成を足すのにコード変更が要らない。

use std::fmt::Write as _;

/// 既定の構成（ADR-0034・ADR-0036）。
const DEFAULT_ARCH: &str = "256x32x32";

/// SIMD実装（`nnue_simd.rs`）が課す倍数の制約。
/// FTはi16を16レーンで回す。隠れ層の出力は4行同時（ROWS=4）に計算し、
/// 最終層は8レーンの内積で畳む。
const FT_MULTIPLE: usize = 16;
const L1_MULTIPLE: usize = 4;
const L2_MULTIPLE: usize = 8;
/// 隠れ層の入力に要る倍数。AVX2が32バイト単位で読むため、
/// L1出力はこの倍数へ切り上げて（ゼロ埋めして）L2へ渡す。
const PAD_MULTIPLE: usize = 32;

/// 次元の上限。桁違いの値を書き間違えたときに、確保量で気づく前に止める。
const MAX_DIM: usize = 4096;

struct Arch {
    ft: usize,
    l1: usize,
    l2: usize,
}

fn parse(spec: &str) -> Result<Arch, String> {
    let parts: Vec<&str> = spec.split('x').collect();
    let [ft, l1, l2] = parts.as_slice() else {
        return Err(format!(
            "`<FT>x<L1>x<L2>` の形で書く（例 512x16x32）。渡された値: {spec}"
        ));
    };
    let num = |name: &str, s: &str| -> Result<usize, String> {
        s.parse::<usize>()
            .map_err(|_| format!("{name}が整数でない: {s}"))
    };
    Ok(Arch {
        ft: num("FT", ft)?,
        l1: num("L1", l1)?,
        l2: num("L2", l2)?,
    })
}

fn validate(a: &Arch) -> Result<(), String> {
    let check = |name: &str, v: usize, m: usize| -> Result<(), String> {
        if v == 0 || !v.is_multiple_of(m) {
            return Err(format!("{name}は{m}の倍数で1以上にする（{v}が渡された）"));
        }
        if v > MAX_DIM {
            return Err(format!("{name}が上限{MAX_DIM}を超えている: {v}"));
        }
        Ok(())
    };
    check("FT", a.ft, FT_MULTIPLE)?;
    check("L1", a.l1, L1_MULTIPLE)?;
    check("L2", a.l2, L2_MULTIPLE)
}

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-env-changed=HIMAWARI_ARCH");
    println!("cargo::rustc-check-cfg=cfg(arch_default)");

    let spec = std::env::var("HIMAWARI_ARCH").unwrap_or_else(|_| DEFAULT_ARCH.to_string());
    let arch = parse(&spec).unwrap_or_else(|e| panic!("HIMAWARI_ARCH: {e}"));
    validate(&arch).unwrap_or_else(|e| panic!("HIMAWARI_ARCH={spec}: {e}"));

    // 既定構成でだけ意味を持つもの（やねうら王形式の読み込みなど）を
    // 切り替えるための目印。
    if spec == DEFAULT_ARCH {
        println!("cargo::rustc-cfg=arch_default");
    }

    let l1_pad = arch.l1.next_multiple_of(PAD_MULTIPLE);
    let mut src = String::new();
    writeln!(src, "/// FT出力次元（片視点）。build.rsが生成する。").unwrap();
    writeln!(src, "pub const FT_OUT: usize = {};", arch.ft).unwrap();
    writeln!(src, "/// 隠れ層1の出力次元。").unwrap();
    writeln!(src, "pub const L1_OUT: usize = {};", arch.l1).unwrap();
    writeln!(src, "/// 隠れ層2の出力次元。").unwrap();
    writeln!(src, "pub const L2_OUT: usize = {};", arch.l2).unwrap();
    writeln!(
        src,
        "/// 隠れ層2へ渡すときの入力幅。L1_OUTを{PAD_MULTIPLE}の倍数へ切り上げ、\n\
         /// 余りはゼロで埋める（SIMDが32バイト単位で読むため）。"
    )
    .unwrap();
    writeln!(src, "pub const L1_PAD: usize = {l1_pad};").unwrap();
    writeln!(src, "/// 構成名。評価ファイルの来歴とログに載せる。").unwrap();
    writeln!(src, "pub const ARCH: &str = {spec:?};").unwrap();

    let out = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("arch.rs");
    std::fs::write(&out, src).unwrap_or_else(|e| panic!("{}が書けない: {e}", out.display()));
}
