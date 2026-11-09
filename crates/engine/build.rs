//! ネットワーク構造の次元をビルド時に決める（ADR-0127）。
//!
//! 環境変数 `HIMAWARI_ARCH` に次元を `x` で区切って渡すと、その構成でビルドする。
//! 要素の数が層の数を決める。
//!
//! ```text
//! 256x16           FT→16→1          （隠れ層1つ）
//! 256x32x32        FT→32→32→1       （隠れ層2つ）
//! 1024x16x32       FT→16→32→1       （隠れ層2つ）
//! 1024x16x32e64    上に、幅64の利きのaccumulatorを足す（ADR-0237）
//! ```
//!
//! 末尾の `e<E>` は、駒×升×利き数の特徴（ADR-0237のF2）を幅Eの別の
//! accumulatorで持つ。省くとE=0で、利きの経路はコードごと消える。
//!
//! `e<E>` の後ろに `-f2_minatt` を書くと、側のaccumulatorの入力をF2と最小の攻め駒の組にする
//! （ADR-0248）。書かなければF2だけを持つ。
//!
//! ```text
//! 256x16x32e64-f2_minatt      F2に最小の攻め駒を足す（ADR-0248。主256で既定、ADR-0261）
//! ```
//!
//! featureで持つと構成の数だけfeatureが要り、組み合わせが積で増える。
//! 1つの文字列で受ければ構成を足すのにコード変更が要らない。

use std::fmt::Write as _;

/// 既定の構成（ADR-0034・ADR-0036、後段はADR-0170、利きはADR-0237、
/// 主の幅と最小の攻め駒はADR-0261）。
const DEFAULT_ARCH: &str = "256x16x32e64-f2_minatt";

/// SIMD実装（`nnue_simd.rs`）が課す倍数の制約。
/// FTはi16を16レーンで回す。隠れ層の出力は4行同時（ROWS=4）に計算し、
/// 最後の層は8レーンの内積で畳む。
const FT_MULTIPLE: usize = 16;
const L1_MULTIPLE: usize = 4;
const L2_MULTIPLE: usize = 8;
/// 隠れ層の入力に要る倍数。AVX2が32バイト単位で読むため、隠れ層の出力は
/// この倍数へ切り上げて（ゼロ埋めして）次の層へ渡す。
const PAD_MULTIPLE: usize = 32;

/// 利きのaccumulatorの幅の倍数。連結ベクトル（HalfKPの活性＋利きの活性
/// 2視点）を32の倍数に保つため、片視点の幅を16の倍数にする。
const EFF_MULTIPLE: usize = 16;

/// 側の入力の種類（ADR-0245）。名前・表の行数・1視点で立つ数の上限。
/// 並びの順が `side_feat` の種類の番号になる。**行数と上限は `side_feat` の
/// 定義と合わせる。** 合っていないとテストが落ちる。
const SIDE_KINDS: &[(&str, usize, usize)] =
    &[("f2", 81 * 29 * 16, 81), ("minatt", 81 * 29 * 11 * 11, 81)];

/// 次元の上限。桁違いの値を書き間違えたときに、確保量で気づく前に止める。
const MAX_DIM: usize = 4096;

struct Arch {
    ft: usize,
    l1: usize,
    /// 隠れ層1つの構成では0。0なら隠れ層1から出力へ直結する。
    l2: usize,
    /// 利きのaccumulatorの幅（ADR-0237）。0なら持たない。
    eff: usize,
    /// 側の入力の種類（`SIDE_KINDS` の番号）。空ならF2の従来の経路。
    eff_kinds: Vec<usize>,
}

fn parse(spec: &str) -> Result<Arch, String> {
    // 末尾の e<E>[-<種類>[_<種類>…]] を先に切り離す。残りは従来の書式のまま読む
    let (body, eff, eff_kinds) = match spec.split_once('e') {
        Some((body, e)) => {
            let (e, kinds) = match e.split_once('-') {
                Some((e, k)) => (e, Some(k)),
                None => (e, None),
            };
            let eff = e
                .parse::<usize>()
                .map_err(|_| format!("利きの幅が整数でない: {e}"))?;
            let kinds = match kinds {
                None => Vec::new(),
                Some(k) => k
                    .split('_')
                    .map(|name| {
                        SIDE_KINDS
                            .iter()
                            .position(|(n, _, _)| *n == name)
                            .ok_or_else(|| format!("側の入力の種類が不明: {name}"))
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            };
            (body, eff, kinds)
        }
        None => (spec, 0, Vec::new()),
    };
    let parts: Vec<&str> = body.split('x').collect();
    let num = |name: &str, s: &str| -> Result<usize, String> {
        s.parse::<usize>()
            .map_err(|_| format!("{name}が整数でない: {s}"))
    };
    let (ft, l1, l2) = match parts.as_slice() {
        [ft, l1] => (*ft, *l1, "0"),
        [ft, l1, l2] => (*ft, *l1, *l2),
        _ => {
            return Err(format!(
                "`<FT>x<L1>[x<L2>][e<E>]` の形で書く（例 256x16、512x16x32、1024x16x32e64）。\
                 渡された値: {spec}"
            ));
        }
    };
    Ok(Arch {
        ft: num("FT", ft)?,
        l1: num("L1", l1)?,
        l2: num("L2", l2)?,
        eff,
        eff_kinds,
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
    if a.eff != 0 {
        check("利きの幅", a.eff, EFF_MULTIPLE)?;
    }
    if !a.eff_kinds.is_empty() && a.eff_kinds != [0, 1] {
        return Err("側の入力の種類は `-f2_minatt` だけを書ける".to_string());
    }
    // 最後の隠れ層は8レーンの内積で畳むので、8の倍数が要る
    let last = if a.l2 != 0 { a.l2 } else { a.l1 };
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
    println!("cargo::rerun-if-env-changed=HIMAWARI_EVAL_HASH_BITS");

    // 配列盤の答えをcoreと照合する構成（ADR-0241）
    println!("cargo::rustc-check-cfg=cfg(mb_shadow)");
    if std::env::var_os("CARGO_FEATURE_MAILBOX_SHADOW").is_some() {
        println!("cargo::rustc-cfg=mb_shadow");
    }

    // eval hashのエントリ数の指数（ADR-0049、issue #429）。既定23は64MBで
    // 従来と同じ。wasmのような小メモリの的に向けてビルド時だけ絞れる。
    // 実行時オプションは設けない方針（ADR-0049）を変えない
    let eval_hash_bits: usize = std::env::var("HIMAWARI_EVAL_HASH_BITS")
        .map(|v| {
            v.parse()
                .unwrap_or_else(|_| panic!("HIMAWARI_EVAL_HASH_BITS が整数でない: {v}"))
        })
        .unwrap_or(23);
    assert!(
        (10..=28).contains(&eval_hash_bits),
        "HIMAWARI_EVAL_HASH_BITS は10〜28で指定する: {eval_hash_bits}"
    );

    let spec = std::env::var("HIMAWARI_ARCH").unwrap_or_else(|_| DEFAULT_ARCH.to_string());
    let arch = parse(&spec).unwrap_or_else(|e| panic!("HIMAWARI_ARCH: {e}"));
    validate(&arch).unwrap_or_else(|e| panic!("HIMAWARI_ARCH={spec}: {e}"));

    let mut src = String::new();
    writeln!(src, "/// eval hashのエントリ数の指数。build.rsが生成する。").unwrap();
    writeln!(src, "pub const EVAL_HASH_BITS: usize = {eval_hash_bits};").unwrap();
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
    writeln!(
        src,
        "/// 利きのaccumulatorの幅（片視点、ADR-0237）。0なら利きの特徴を持たない。"
    )
    .unwrap();
    writeln!(src, "pub const EFF_OUT: usize = {};", arch.eff).unwrap();
    // 側の入力（ADR-0245）。種類を書かなければF2だけの従来の経路
    let generic = !arch.eff_kinds.is_empty();
    let kinds = if arch.eff_kinds.is_empty() {
        vec![0]
    } else {
        arch.eff_kinds.clone()
    };
    let eff_in: usize = kinds.iter().map(|&k| SIDE_KINDS[k].1).sum();
    let eff_max: usize = kinds.iter().map(|&k| SIDE_KINDS[k].2).sum();
    writeln!(
        src,
        "/// 側の入力をF2と最小の攻め駒の組で持つか（ADR-0248）。\n\
         /// 偽ならF2だけを差分で持つ従来の経路（ADR-0237・0242）。"
    )
    .unwrap();
    writeln!(src, "pub const EFF_GENERIC: bool = {generic};").unwrap();
    writeln!(
        src,
        "/// 側の入力の種類の番号（`side_feat::KIND_*`）。並びの順に表の行が続く。"
    )
    .unwrap();
    writeln!(src, "pub const EFF_KINDS: &[usize] = &{kinds:?};").unwrap();
    writeln!(src, "/// 側の特徴の総数（表の行数）。").unwrap();
    writeln!(src, "pub const EFF_IN: usize = {eff_in};").unwrap();
    writeln!(
        src,
        "/// 1視点で立つ側の特徴の数の上限。学習側はこの数へ詰め物をして揃える。"
    )
    .unwrap();
    writeln!(src, "pub const EFF_MAX_ACTIVE: usize = {eff_max};").unwrap();
    writeln!(src, "/// 構成名。評価ファイルの来歴とログに載せる。").unwrap();
    writeln!(src, "pub const ARCH: &str = {spec:?};").unwrap();

    let out = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("arch.rs");
    std::fs::write(&out, src).unwrap_or_else(|e| panic!("{}が書けない: {e}", out.display()));
}
