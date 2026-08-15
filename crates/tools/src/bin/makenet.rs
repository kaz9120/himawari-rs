//! NNUEネット生成・変換ツール（ADR-0037）。
//!
//! 使い方:
//!   makenet [--seed N] [--out path]           乱数ネット生成（配線検証・ベンチ用）
//!   makenet --import nn.bin [--out path]      やねうら王形式HalfKPネットを
//!                                             独自形式へ変換（利き塔ゼロ）
//!   makenet --resize other.hmwr [--out path]  別の構成のネットを、いまの
//!                                             ビルド構成へ合わせる
//!   makenet --reorder perm.txt --from net.hmwr [--out path]
//!                                             FT出力次元を並べ替える
//!
//! 学習器が作らないネットを用意する役目で、現行の実験でも使う。乱数ネットは
//! 配線検証と学習の下限測定（ADR-0133）、`--resize` はネット形状の速度比較
//! （ADR-0127）と継続学習の構成合わせ（ADR-0130）、`--reorder` は第1層の
//! 列駆動が飛ばせるチャンクを増やす並べ替え（ADR-0168）から呼ぶ。

use himawari_engine::nnue::{CONCAT, FT_OUT, NnueNetwork};
use himawari_engine::nnue_io::{load, load_resized, save};

/// nn.bin（やねうら王形式）を読む。FT 256専用（ADR-0067）。
fn import_net(path: &str) -> (NnueNetwork, String) {
    let mut f = std::fs::File::open(path).unwrap_or_else(|e| {
        eprintln!("開けません: {path}: {e}");
        std::process::exit(1);
    });
    let (net, arch) = himawari_engine::nnue_compat::load_nn_bin(&mut f).unwrap_or_else(|e| {
        eprintln!("nn.bin読み込み失敗: {e}");
        std::process::exit(1);
    });
    (net, format!("imported from {path} ({arch})"))
}

/// 別の構成のネットを、いまのビルド構成へ合わせる（ADR-0127）。
/// 広げる向きなら評価値が完全に一致するので、構成だけを変えて探索木を
/// 揃えた速度比較ができる。
fn resize_net(path: &str) -> (NnueNetwork, String) {
    let mut f = std::fs::File::open(path).unwrap_or_else(|e| {
        eprintln!("開けません: {path}: {e}");
        std::process::exit(1);
    });
    load_resized(&mut f).unwrap_or_else(|e| {
        eprintln!("構成の変換に失敗: {e}");
        std::process::exit(1);
    })
}

/// 置換を読む。1行1整数で、`0..FT_OUT` の順列であることを確かめる。
fn read_perm(path: &str) -> Vec<usize> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("開けません: {path}: {e}");
        std::process::exit(1);
    });
    let perm: Vec<usize> = text
        .split_whitespace()
        .map(|s| {
            s.parse().unwrap_or_else(|_| {
                eprintln!("置換に整数でない値がある: {s}");
                std::process::exit(1);
            })
        })
        .collect();
    if perm.len() != FT_OUT {
        eprintln!("置換の長さが違う: {} 個（FT_OUT={FT_OUT}）", perm.len());
        std::process::exit(1);
    }
    let mut seen = vec![false; FT_OUT];
    for &p in &perm {
        if p >= FT_OUT || seen[p] {
            eprintln!("置換が順列になっていない: {p}");
            std::process::exit(1);
        }
        seen[p] = true;
    }
    perm
}

/// FT出力次元を並べ替える（ADR-0168）。
///
/// 新しい位置jへ元の次元 `perm[j]` を移す。`ft_b`・`ft_w` の列・`w2` の
/// 入力列（両視点）へ同じ置換を入れるので、**積和の項の集合は変わらず
/// 評価値はビット一致する。** ゼロになりやすい次元を同じ4列チャンクへ
/// 寄せると、第1層の列駆動が飛ばせるチャンクが増える。
fn apply_perm(net: NnueNetwork, perm: &[usize]) -> NnueNetwork {
    let mut out = net;
    out.ft_b = perm.iter().map(|&p| out.ft_b[p]).collect();
    let mut ft_w = vec![0 as himawari_engine::nnue::FtWeight; out.ft_w.len()];
    for feature in 0..out.ft_w.len() / FT_OUT {
        let base = feature * FT_OUT;
        for (j, &p) in perm.iter().enumerate() {
            ft_w[base + j] = out.ft_w[base + p];
        }
    }
    let mut w2 = vec![0i8; out.w2.len()];
    for row in 0..out.w2.len() / CONCAT {
        let base = row * CONCAT;
        for (j, &p) in perm.iter().enumerate() {
            // 視点0と視点1へ同じ置換を入れる
            w2[base + j] = out.w2[base + p];
            w2[base + FT_OUT + j] = out.w2[base + FT_OUT + p];
        }
    }
    out.ft_w = ft_w;
    out.w2 = w2;
    out.finish()
}

/// 置換ファイルを読み、元ネットへ当てる。
fn reorder_net(path: &str, perm_path: &str) -> (NnueNetwork, String) {
    let perm = read_perm(perm_path);
    let mut f = std::fs::File::open(path).unwrap_or_else(|e| {
        eprintln!("開けません: {path}: {e}");
        std::process::exit(1);
    });
    let (net, lineage) = load(&mut f).unwrap_or_else(|e| {
        eprintln!("読み込み失敗: {e}");
        std::process::exit(1);
    });
    let net = apply_perm(net, &perm);
    (net, format!("{lineage} + reordered by {perm_path}"))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut seed = 1u64;
    let mut out = "random.hmwr".to_string();
    let mut import: Option<String> = None;
    let mut resize: Option<String> = None;
    let mut reorder: Option<String> = None;
    let mut from: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--seed" => {
                i += 1;
                seed = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(1);
            }
            "--out" => {
                i += 1;
                out = args.get(i).cloned().unwrap_or(out);
            }
            "--import" => {
                i += 1;
                import = args.get(i).cloned();
                if import.is_none() {
                    eprintln!("--import にはnn.binのパスが必要です");
                    std::process::exit(1);
                }
            }
            "--resize" => {
                i += 1;
                resize = args.get(i).cloned();
                if resize.is_none() {
                    eprintln!("--resize には元になる.hmwrのパスが必要です");
                    std::process::exit(1);
                }
            }
            "--reorder" => {
                i += 1;
                reorder = args.get(i).cloned();
                if reorder.is_none() {
                    eprintln!("--reorder には置換ファイルのパスが必要です");
                    std::process::exit(1);
                }
            }
            "--from" => {
                i += 1;
                from = args.get(i).cloned();
                if from.is_none() {
                    eprintln!("--from には元になる.hmwrのパスが必要です");
                    std::process::exit(1);
                }
            }
            other => {
                eprintln!("不明な引数: {other}");
                std::process::exit(1);
            }
        }
        i += 1;
    }
    let (net, lineage) = match (&import, &resize, &reorder) {
        (Some(path), None, None) => import_net(path),
        (None, Some(path), None) => resize_net(path),
        (None, None, Some(perm)) => {
            let Some(src) = from.as_deref() else {
                eprintln!("--reorder には --from で元の.hmwrを渡してください");
                std::process::exit(1);
            };
            reorder_net(src, perm)
        }
        (None, None, None) => (NnueNetwork::random(seed), format!("random seed={seed}")),
        _ => {
            eprintln!("--import・--resize・--reorder は同時に指定できません");
            std::process::exit(1);
        }
    };
    let mut f = std::fs::File::create(&out).unwrap_or_else(|e| {
        eprintln!("作成できません: {e}");
        std::process::exit(1);
    });
    save(&net, &lineage, &mut f).unwrap_or_else(|e| {
        eprintln!("書き出し失敗: {e}");
        std::process::exit(1);
    });
    println!("{out} を書き出しました ({lineage})");
}

#[cfg(test)]
mod tests {
    use super::*;
    use himawari_engine::nnue::FT_IN;
    use himawari_engine::nnue_simd::{forward_hidden, ft_refresh};

    /// 疑似乱数（xorshift）。テスト内で再現できれば足りる。
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
    }

    /// 4個ずつずらす置換。並びが変わればよいので中身は問わない。
    fn rotate_perm() -> Vec<usize> {
        (0..FT_OUT).map(|i| (i + 4) % FT_OUT).collect()
    }

    /// 並べ替えたネットは、同じ置換を当てた活性に対して同じ評価値を返す
    /// こと（ADR-0168）。**この一致が並べ替えの前提そのものである。**
    #[test]
    fn reordering_keeps_the_value() {
        let net = NnueNetwork::random(7).finish();
        let perm = rotate_perm();

        let mut rng = Rng(12345);
        let mut concat = [0u8; CONCAT];
        for c in concat.iter_mut() {
            // 4回に3回はゼロにして、実際の活性の疎さに寄せる
            let v = rng.next();
            *c = if v.is_multiple_of(4) {
                (v >> 8) as u8 & 127
            } else {
                0
            };
        }
        let mut moved = [0u8; CONCAT];
        for (j, &p) in perm.iter().enumerate() {
            moved[j] = concat[p];
            moved[FT_OUT + j] = concat[FT_OUT + p];
        }

        let before = forward_hidden(&net, &concat);
        let after = forward_hidden(&apply_perm(net, &perm), &moved);
        assert_eq!(before, after);
    }

    /// FT側の並べ替えが、accumulatorを同じ置換で並べ替えた状態にする
    /// こと。上のテストが要求する「同じ置換を当てた活性」を作る側になる。
    #[test]
    fn reordering_moves_the_accumulator() {
        let net = NnueNetwork::random(9).finish();
        let perm = rotate_perm();
        let features: Vec<u32> = [3u32, 1000, 54321, 98765]
            .iter()
            .map(|&f| f % (FT_IN as u32))
            .collect();

        let mut before = [0i16; FT_OUT];
        ft_refresh(&mut before, &net.ft_b, &net.ft_w, &features);
        let moved_net = apply_perm(net, &perm);
        let mut after = [0i16; FT_OUT];
        ft_refresh(&mut after, &moved_net.ft_b, &moved_net.ft_w, &features);

        for (j, &p) in perm.iter().enumerate() {
            assert_eq!(after[j], before[p], "次元{j}が元の{p}になっていない");
        }
    }
}
