//! ネットワーク構造の次元をビルド時に決める（ADR-0127）。
//!
//! 環境変数 `HIMAWARI_ARCH` に次元を `x` で区切って渡すと、その構成でビルドする。
//! 要素の数が層の数を決める。
//!
//! ```text
//! 256x16           FT→16→1          （隠れ層1つ）
//! 256x32x32        FT→32→32→1       （隠れ層2つ。既定）
//! 256x32x32x32     FT→32→32→32→1    （隠れ層3つ）
//! ```
//!
//! featureで持つと構成の数だけfeatureが要り、組み合わせが積で増える。
//! 1つの文字列で受ければ構成を足すのにコード変更が要らない。

use std::fmt::Write as _;

/// 既定の構成（ADR-0034・ADR-0036）。
const DEFAULT_ARCH: &str = "256x32x32";

/// SIMD実装（`nnue_simd.rs`）が課す倍数の制約。
/// FTはi16を16レーンで回す。隠れ層の出力は4行同時（ROWS=4）に計算し、
/// 最後の層は8レーンの内積で畳む。
const FT_MULTIPLE: usize = 16;
const L1_MULTIPLE: usize = 4;
const L2_MULTIPLE: usize = 8;
const L3_MULTIPLE: usize = 8;
/// 隠れ層の入力に要る倍数。AVX2が32バイト単位で読むため、隠れ層の出力は
/// この倍数へ切り上げて（ゼロ埋めして）次の層へ渡す。
const PAD_MULTIPLE: usize = 32;

/// 次元の上限。桁違いの値を書き間違えたときに、確保量で気づく前に止める。
const MAX_DIM: usize = 4096;

struct Arch {
    ft: usize,
    l1: usize,
    /// 隠れ層1つの構成では0。0なら隠れ層1から出力へ直結する。
    l2: usize,
    /// 隠れ層2つ以下の構成では0。
    l3: usize,
}

fn parse(spec: &str) -> Result<Arch, String> {
    let parts: Vec<&str> = spec.split('x').collect();
    let num = |name: &str, s: &str| -> Result<usize, String> {
        s.parse::<usize>()
            .map_err(|_| format!("{name}が整数でない: {s}"))
    };
    let (ft, l1, l2, l3) = match parts.as_slice() {
        [ft, l1] => (*ft, *l1, "0", "0"),
        [ft, l1, l2] => (*ft, *l1, *l2, "0"),
        [ft, l1, l2, l3] => (*ft, *l1, *l2, *l3),
        _ => {
            return Err(format!(
                "`<FT>x<L1>[x<L2>[x<L3>]]` の形で書く（例 256x16、512x16x32）。\
                 渡された値: {spec}"
            ));
        }
    };
    Ok(Arch {
        ft: num("FT", ft)?,
        l1: num("L1", l1)?,
        l2: num("L2", l2)?,
        l3: num("L3", l3)?,
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
    // 書かなかった層は0で、そのぶん層が減る
    if a.l2 != 0 {
        check("L2", a.l2, L2_MULTIPLE)?;
    }
    if a.l3 != 0 {
        check("L3", a.l3, L3_MULTIPLE)?;
    }
    if a.l2 == 0 && a.l3 != 0 {
        return Err("L2を書かずにL3だけ書くことはできない".to_string());
    }
    // 最後の隠れ層は8レーンの内積で畳むので、8の倍数が要る
    let last = if a.l3 != 0 {
        a.l3
    } else if a.l2 != 0 {
        a.l2
    } else {
        a.l1
    };
    if !last.is_multiple_of(L2_MULTIPLE) {
        return Err(format!(
            "最後の隠れ層は{L2_MULTIPLE}の倍数にする（{last}が渡された）"
        ));
    }
    Ok(())
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

    let mut src = String::new();
    writeln!(src, "/// FT出力次元（片視点）。build.rsが生成する。").unwrap();
    writeln!(src, "pub const FT_OUT: usize = {};", arch.ft).unwrap();
    writeln!(src, "/// 隠れ層1の出力次元。").unwrap();
    writeln!(src, "pub const L1_OUT: usize = {};", arch.l1).unwrap();
    writeln!(
        src,
        "/// 隠れ層2の出力次元。0なら隠れ層1から出力へ直結する。"
    )
    .unwrap();
    writeln!(src, "pub const L2_OUT: usize = {};", arch.l2).unwrap();
    writeln!(
        src,
        "/// 隠れ層3の出力次元。0なら層を挟まず、隠れ層2から出力へ直結する。"
    )
    .unwrap();
    writeln!(src, "pub const L3_OUT: usize = {};", arch.l3).unwrap();
    writeln!(
        src,
        "/// 次の層へ渡すときの入力幅。{PAD_MULTIPLE}の倍数へ切り上げ、余りはゼロで\n\
         /// 埋める（SIMDが32バイト単位で読むため）。最後の層へ渡すぶんは\n\
         /// 8レーンの内積で畳むので切り上げない。"
    )
    .unwrap();
    let l1_pad = if arch.l2 == 0 {
        arch.l1
    } else {
        arch.l1.next_multiple_of(PAD_MULTIPLE)
    };
    writeln!(src, "pub const L1_PAD: usize = {l1_pad};").unwrap();
    // 次の層へ渡すぶんだけ切り上げる。最後の隠れ層は内積で畳むので
    // 切り上げない（余分なゼロ列を持たない）
    let l2_pad = if arch.l3 == 0 {
        arch.l2
    } else {
        arch.l2.next_multiple_of(PAD_MULTIPLE)
    };
    writeln!(src, "pub const L2_PAD: usize = {l2_pad};").unwrap();
    writeln!(src, "/// 構成名。評価ファイルの来歴とログに載せる。").unwrap();
    writeln!(src, "pub const ARCH: &str = {spec:?};").unwrap();

    let out = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("arch.rs");
    std::fs::write(&out, src).unwrap_or_else(|e| panic!("{}が書けない: {e}", out.display()));
}
