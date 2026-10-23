//! ネットワーク構造の次元をビルド時に決める（ADR-0127）。
//!
//! 環境変数 `HIMAWARI_ARCH` に次元を `x` で区切って渡すと、その構成でビルドする。
//! 要素の数が層の数を決める。
//!
//! ```text
//! 256x16           FT→16→1          （隠れ層1つ）
//! 256x32x32        FT→32→32→1       （隠れ層2つ）
//! 1024x16x32       FT→16→32→1       （隠れ層2つ）
//! 256x32x32x32     FT→32→32→32→1    （隠れ層3つ）
//! 1024x16x32e64    上に、幅64の利きのaccumulatorを足す（ADR-0237。既定）
//! ```
//!
//! 末尾の `e<E>` は、駒×升×利き数の特徴（ADR-0237のF2）を幅Eの別の
//! accumulatorで持つ。省くとE=0で、利きの経路はコードごと消える。
//!
//! 側のaccumulatorの入力と活性は、`e<E>` の後ろで替えられる（ADR-0245）。
//!
//! ```text
//! 1024x16x32e64p              活性に対の積を掛ける（片視点E/2を出す）
//! 1024x16x32e64-thermo        入力を温度計の形にする
//! 1024x16x32e64-f2_threat     F2に脅威の特徴を足す
//! 1024x16x32e1024p-f2_hand    幅1024で対の積を掛け、F2と持ち駒を入れる
//! ```
//!
//! 種類は `_` でつなぐ（構成名はビルドやネットの名前に入るので、`hmwr` が名前に許す文字に限る）。
//! 表し方を書かない `e<E>` はF2で、従来の差分の経路を通る。表し方か `p` を書くと、
//! 評価のたびに作り直す経路（`side_feat`）を通る。
//!
//! featureで持つと構成の数だけfeatureが要り、組み合わせが積で増える。
//! 1つの文字列で受ければ構成を足すのにコード変更が要らない。

use std::fmt::Write as _;

/// 既定の構成（ADR-0034・ADR-0036、幅はADR-0159、後段はADR-0170、利きはADR-0237）。
const DEFAULT_ARCH: &str = "1024x16x32e64";

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

/// 利きのaccumulatorの幅の倍数。連結ベクトル（HalfKPの活性＋利きの活性
/// 2視点）を32の倍数に保つため、片視点の幅を16の倍数にする。
const EFF_MULTIPLE: usize = 16;

/// 側の入力の種類（ADR-0245）。名前・表の行数・1視点で立つ数の上限。
/// 並びの順が `side_feat` の種類の番号になる。**行数と上限は `side_feat` の
/// 定義と合わせる。** 合っていないとテストが落ちる。
const SIDE_KINDS: &[(&str, usize, usize)] = &[
    ("f2", 81 * 29 * 16, 81),
    ("l2", 81 * 29 * 4, 81),
    ("thermo", 81 * 29 + 81 * 2 * 3, 81 + 81 * 2 * 3),
    ("minatt", 81 * 29 * 11 * 11, 81),
    ("threat", 28 * 81 * 11 * 2, 38),
    ("escape", 2 * 8 * 5, 16),
    ("drop", 2 * 7 * 4, 2 * 7 * 4),
    ("mobility", 28 * 8, 38 * 8),
    ("kw5", 2 * 29 * 16 * 25, 50),
    ("hand", 2 * 7 * 18, 38),
];

/// 次元の上限。桁違いの値を書き間違えたときに、確保量で気づく前に止める。
const MAX_DIM: usize = 4096;

struct Arch {
    ft: usize,
    l1: usize,
    /// 隠れ層1つの構成では0。0なら隠れ層1から出力へ直結する。
    l2: usize,
    /// 隠れ層2つ以下の構成では0。
    l3: usize,
    /// 利きのaccumulatorの幅（ADR-0237）。0なら持たない。
    eff: usize,
    /// 側のaccumulatorの活性に対の積を掛けるか（ADR-0245）。
    eff_pair: bool,
    /// 側の入力の種類（`SIDE_KINDS` の番号）。空ならF2の従来の経路。
    eff_kinds: Vec<usize>,
}

fn parse(spec: &str) -> Result<Arch, String> {
    // 末尾の e<E>[p][-<種類>[_<種類>…]] を先に切り離す。残りは従来の書式のまま読む
    let (body, eff, eff_pair, eff_kinds) = match spec.split_once('e') {
        Some((body, e)) => {
            let (e, kinds) = match e.split_once('-') {
                Some((e, k)) => (e, Some(k)),
                None => (e, None),
            };
            let (e, pair) = match e.strip_suffix('p') {
                Some(e) => (e, true),
                None => (e, false),
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
            (body, eff, pair, kinds)
        }
        None => (spec, 0, false, Vec::new()),
    };
    let parts: Vec<&str> = body.split('x').collect();
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
                "`<FT>x<L1>[x<L2>[x<L3>]][e<E>]` の形で書く（例 256x16、512x16x32、1024x16x32e64）。\
                 渡された値: {spec}"
            ));
        }
    };
    Ok(Arch {
        ft: num("FT", ft)?,
        l1: num("L1", l1)?,
        l2: num("L2", l2)?,
        l3: num("L3", l3)?,
        eff,
        eff_pair,
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
    if a.l3 != 0 {
        check("L3", a.l3, L3_MULTIPLE)?;
    }
    if a.eff != 0 {
        check("利きの幅", a.eff, EFF_MULTIPLE)?;
    }
    // 対の積は片視点E/2を出す。連結ベクトルを32の倍数に保つには、E/2も16の倍数が要る
    if a.eff_pair && !a.eff.is_multiple_of(2 * EFF_MULTIPLE) {
        return Err(format!(
            "対の積を掛ける側の幅は{}の倍数にする（{}が渡された）",
            2 * EFF_MULTIPLE,
            a.eff
        ));
    }
    let mut seen = a.eff_kinds.clone();
    seen.sort_unstable();
    seen.dedup();
    if seen.len() != a.eff_kinds.len() {
        return Err("側の入力の種類が重複している".to_string());
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
    println!("cargo::rerun-if-env-changed=HIMAWARI_EVAL_HASH_BITS");

    // 探索の盤の構成（ADR-0241）。フィーチャは足し算でしか合わさらないので、
    // coreの探索を選ぶ core-search を、配列盤の mailbox より優先させる。
    // ワークスペースで一緒にビルドしたほかのクレートが core-search を有効にすれば、
    // そのビルドの探索はcoreになる
    println!("cargo::rustc-check-cfg=cfg(mb_search)");
    println!("cargo::rustc-check-cfg=cfg(mb_shadow)");
    let feature = |name: &str| std::env::var_os(format!("CARGO_FEATURE_{name}")).is_some();
    if feature("MAILBOX") && !feature("CORE_SEARCH") {
        println!("cargo::rustc-cfg=mb_search");
        if feature("MAILBOX_SHADOW") {
            println!("cargo::rustc-cfg=mb_shadow");
        }
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
    writeln!(
        src,
        "/// 利きのaccumulatorの幅（片視点、ADR-0237）。0なら利きの特徴を持たない。"
    )
    .unwrap();
    writeln!(src, "pub const EFF_OUT: usize = {};", arch.eff).unwrap();
    // 側の入力（ADR-0245）。種類を書かなければF2だけで、対の積も掛けない従来の経路
    let generic = arch.eff_pair || !arch.eff_kinds.is_empty();
    let kinds = if arch.eff_kinds.is_empty() {
        vec![0]
    } else {
        arch.eff_kinds.clone()
    };
    let eff_in: usize = kinds.iter().map(|&k| SIDE_KINDS[k].1).sum();
    let eff_max: usize = kinds.iter().map(|&k| SIDE_KINDS[k].2).sum();
    writeln!(
        src,
        "/// 側のaccumulatorの入力を、評価のたびに作り直す経路で持つか（ADR-0245）。\n\
         /// 偽ならF2を差分で持つ従来の経路（ADR-0237・0242）。"
    )
    .unwrap();
    writeln!(src, "pub const EFF_GENERIC: bool = {generic};").unwrap();
    writeln!(
        src,
        "/// 側のaccumulatorの活性に対の積を掛けるか（ADR-0245）。"
    )
    .unwrap();
    writeln!(src, "pub const EFF_PAIR: bool = {};", arch.eff_pair).unwrap();
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
