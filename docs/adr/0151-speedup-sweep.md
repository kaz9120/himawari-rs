# 0151: 挙動を変えない高速化の第2弾をプロファイル起点で洗い出す

- Status: proposed
- Date: 2026-08-09
- 関連ADR: [0056](0056-tt-prefetch.md), [0074](0074-feature-verification.md), [0099](0099-nnue-dot-sdot.md), [0101](0101-movelist-uninit.md), [0124](0124-hot-path-allocs.md), [0138](0138-ft-i8-quantization.md), [0147](0147-effect-bucket-features.md), [0148](0148-effect-table.md)

## Context

自己生成教師の第1世代は棄却になり（[ADR-0144](0144-selfplay-teacher-loop.md)、
−35.7 Elo）、学習側の次の一手は探索段階にある。その間に、副作用のない速度
改善を積む（2026-08-09オーナー指示）。[ADR-0124](0124-hot-path-allocs.md)の
枠の続きで、**評価値もノード数も1ビットも変えない改善だけ**を扱う。1件0.5%
でも、群でまとめれば測れる。

### プロファイルの取り直し

深さ25・1スレッド・2000Hzで4局面を読ませた（Apple M4 Pro、FT重みi8ビルド、
17,749サンプル＝約8.9秒）。ADR-0124の時と比べ、インライン化の境界が変わって
`evaluate` にFT差分更新が合算されている。行単位では前回と同じ内訳になる。

| 箇所 | self時間 | 中身（行単位の内訳） |
|---|---|---|
| `NnueState::evaluate` | 35.4% | FT差分更新24.0%（`ensure`）、隠れ層10.3%（`forward_hidden`）、clip 0.8% |
| `MovePicker::next` | 15.0% | `score_quiets` 5.2%、`partial_insertion_sort` 5.6%ほか |
| `Worker::search` | 9.9% | TT probe行が2.7%、ほか分散 |
| `attacks::attacks` | 7.2% | 飛び利きのレイ計算と駒種ディスパッチ |
| `Position::attackers_to` | 6.7% | see_ge・mate_1ply・update_check_infoから |
| `movegen::generate` 系 | 6.7% | `generate` 3.8%＋`generate_board_moves` 1.6%＋`push_variants` 1.3% |
| `libsystem_malloc` | 2.1% | 呼び出し元は下の表 |
| `mate_1ply` | 2.0% | |
| `see_ge` | 1.9% | |
| `_platform_memmove` | 1.2% | ほぼNNUEの `copy_from_slice` |
| `slider_blockers` | 1.4% | |
| `is_legal` | 1.2% | |
| `eval_cached` | 1.2% | eval hashのprobe/store |

### システムライブラリの呼び出し元

leafがシステムライブラリのサンプルを、スタックを遡って自前クレートの
フレームへ帰属させた。**mallocの2.1%は3か所に割れる。**

| leaf | 呼び出し元 | 時間 |
|---|---|---|
| malloc | `Worker::search`（`quiets_searched` / `captures_searched` / `child_pv` の `Vec::new`） | 0.70% |
| malloc | `MovePicker::new`（`Vec::with_capacity(64)`） | 0.50% |
| malloc | `RawVec<ExtMove>::grow_one`（64を超えるノードでの再確保） | 0.41% |
| malloc | `RawVec<Move>::grow_one`（`quiets_searched` 等の成長） | 0.29% |
| malloc | `Worker::qsearch`・`new_probcut` | 0.21% |
| memmove | `NnueState::evaluate`（`ensure` の `copy_from_slice`） | 1.08% |
| memmove | `RawVec` の成長コピー | 0.12% |
| mach_absolute_time | `Worker::search` | 0.27% |

## 枠の条件（ADR-0124を引き継ぐ）

- 機能検証（[ADR-0074](0074-feature-verification.md)）で全局面のノード数が
  一致すること。一致しない案はこの枠で扱わない
- プロファイルに根拠があること
- 群でまとめてNPSで測る。個々の候補は0.2〜1%で誤差（±1.5%）に埋もれる
- 測定はADR-0124と同じ。定跡生成などを止めた静かな環境で7周、分布の分離を見る
- コミットはfix型。SPRTにはかけない

## 候補

群の番号はこのADR内で振り直す（ADR-0124の第1〜3群は実装済み）。

### 群A: NNUEの差分適用を1パスに融合する（最大の枠）

FT差分更新は24.0%で、その本体はaccumulator（256要素×i16＝512バイト）への
読み書きである。現状は1手の適用で最大6回accを往復する。

| 現状のパス | 回数 |
|---|---|
| `ensure` の `copy_from_slice`（親→自分の複製） | 1 |
| `apply_dirty` の `ft_sub`（移動元を引く） | 1 |
| `apply_dirty` の `ft_add`（移動先を足す） | 1 |
| 取る手はさらに `ft_sub`＋手駒の `ft_add` | 0〜2 |

これを「親のaccを読みながら、全差分を足し引きして自分のaccへ書く」1パスに
融合する。i16のラップ加減算は可換かつ結合的なので、**順序を変えても結果は
ビット一致する。** memmoveの1.08%が消え、accの往復が最大6回から1回になる。

`refresh_top` も同様に、38特徴を1行ずつ足す代わりに2〜4行まとめて足し、
accへの読み書き回数を削る。

検証は `incremental_matches_full_computation`（差分＝全計算の完全一致）と
機能検証。[ADR-0147](0147-effect-bucket-features.md)のEffectBucketは差分更新の
コスト増を代償に持つ設計なので、**この融合はその土台の整備を兼ねる。**

### 群B: 探索本体のヒープ確保を消す（malloc 2.1%）

ADR-0124第1群の続き。残っていた確保が呼び出し元つきで特定できた。

| 候補 | 時間 | 中身 |
|---|---|---|
| `search` の `quiets_searched` / `captures_searched` / `child_pv` | 0.70%＋成長0.29% | 毎ノード `Vec::new` し、push時に確保・再確保する。固定長バッファ（インライン配列）へ置き換える |
| `MovePicker` の `moves` | 0.50%＋成長0.41%＋memmove 0.10% | 毎ノード `Vec::with_capacity(64)` を確保し、64手を超えると再確保する。`Worker` が持つply別バッファを貸し出す形にし、容量は `MOVE_LIST_CAP` で固定する |
| `qsearch`・`new_probcut` の同種 | 0.21% | 上と同じ方式で消える |

スタックに置くと `MAX_PLY` の再帰で数百KB積むというADR-0124の懸念は、
バッファをヒープ上のply別領域（`Box`）にすれば起きない。合計約2%で、
この枠では見込みがいちばん確実に立つ群になる。

### 群C: 隠れ層のロード削減（forward_hidden 10.3%）

| 候補 | 中身 |
|---|---|
| 行束ねを4→8へ | SDOT版（[ADR-0099](0099-nnue-dot-sdot.md)）は4行同時で、入力ベクトルのロードを行グループごとに繰り返す。aarch64はSIMDレジスタ32本なので8行同時にでき、入力のロードが半減する |
| 出力層 `dot` のSDOT化 | 出力層は1行なので4行同時の対象外のまま、`Simd<i32, 8>` の逐次拡張で回っている。SDOTなら1命令16要素になる |

積和の順序は変わるが、i32の範囲で飽和しないため結果は一致する
（`forward_hidden_matches_scalar` がスカラーとのビット一致を要求する）。

### 群D: 利き・生成の細部

| 候補 | 時間 | 中身 |
|---|---|---|
| 二歩マスクのビット演算化 | 未計測 | `generate_drops` が9筋をループし、筋ごとに歩の有無を分岐で見る。縦foldのビット演算で一括計算できる。分岐が消える |
| `see_ge` の最安攻撃駒選択 | 1.9%の内側 | 攻撃駒の全マスを走査し `piece_on`＋価値比較で最安を選ぶ。駒種ビットボードを価値昇順に見る方式なら走査が消える。**同価値の駒が複数あるときの選択（最小マス番号）を現行と一致させることが条件。** 一致しないと取り合いの展開が変わり、この枠から外れる |
| `see_ge` の攻撃駒差分更新 | 同上 | 取り合いのループが毎回 `attackers_to` を全計算する。取り除いたマスの背後のX線だけ足す差分にできる。[ADR-0148](0148-effect-table.md)の利きテーブルが入ればそちらに置き換わる |

### 群E: Sharedのfalse sharing（別枠、Threads=4で測る）

ADR-0124から持ち越し。アトミック群が同一キャッシュラインに乗っている。
1スレッドでは効果がゼロなので、既定条件のNPS測定に混ぜず別枠のまま残す。

## 候補から外すもの（記録）

| 案 | 外す理由 |
|---|---|
| `partial_insertion_sort`（5.6%） | 最終的な並びを完全一致させたまま速くする実装が自明でない。挿入シフトの `copy_within` 化は並びを保つが、利得の見込みが立たない。低確度のまま保留 |
| `score_quiets` のhistory読み（5.2%） | 巨大表へのランダムアクセスでキャッシュミスが主因。prefetchはこの環境で効かないことを実測済み（[ADR-0056](0056-tt-prefetch.md)・ADR-0124） |
| `attacks()` の駒種単相化 | 指し手の生成順が変わり、ムーブオーダリングが変わる。ADR-0124の判断を維持する |
| TTエントリの圧縮・レイアウト変更 | 置換ポリシーが変わり、探索が変わる。この枠の外 |
| FTのさらなる量子化 | 評価値が変わる。[ADR-0138](0138-ft-i8-quantization.md)の枠 |
| `mach_absolute_time`（0.27%） | 時刻取得は2048ノードに1回のはずで、この量は計算が合わない。0.3%なので改善でなく原因調査だけをTODOに残す |

## Decision

群A〜Dを実装する。順序は見込みの大きさで決め、A（NNUE融合）→B（ヒープ確保）
→C（隠れ層）→Dとする。群ごとに1PR・1回のNPS測定とし、どの群も機能検証で
全局面のノード数一致を通過してからマージする。群Eは `Threads=4` の測定枠を
用意できたときに別PRで扱う。

見込みの合計は3〜5%になる。根拠は、mallocとmemmoveの実測3.3%のうち大半が
群A・Bで消えること、FT更新24.0%のメモリ往復が最大6分の1になることである。
ただしADR-0124で第2群の見込みが外れた前例があるので、群単位の実測で判断する。

## Consequences

- 群でまとめる代償として、群の中のどの候補がどれだけ効いたかは分からない。
  ADR-0124と同じ割り切りを繰り返す
- 群Aは `nnue_acc.rs` のデータフローを変えるリファクタリングになる。完全一致
  テストと機能検証が安全網で、これを崩す変更はこの枠に入れない
- 群Aの融合は[ADR-0147](0147-effect-bucket-features.md)のEffectBucketが要求する
  差分更新コストの削減を先取りする。EffectBucketの実測判断の前提が良い方へ動く
- 群Dの `see_ge` 2件は「一致させる条件」が付く。機能検証で1ノードでもずれたら
  案ごと取り下げ、挙動を変える改善としてSPRT枠へ回すかを別途判断する
- NPSが群単位でしか残らないので、後から個別の寄与を知りたくなったら測り直しに
  なる
