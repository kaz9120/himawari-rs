# 0042: 学習ログと実験レジストリ

- Status: rejected（ADR-0040のPyTorch移行により、TensorBoard等で代替）
- Date: 2026-07-20
- 関連ADR: [0039](0039-trainer-v1.md), [0040](0040-training-infra-v2.md)

## Context

現在の学習器は標準エラーにログを出すだけで、構造化された
記録がない。valid lossの推移をプロットするにはログを手動で
加工する必要がある。学習実行ごとのハイパラ・結果の比較も
手作業になる。

P6でlr scheduleやデータ量を変えた実験を複数回す。条件と
結果を系統的に記録・比較する仕組みが要る。

## Decision

### 学習ログ: TSV出力

--log-file PATH で学習メトリクスをTSVファイルに書き出す。
指定なしでは標準エラーのみ（現行動作を維持）。

列定義:

```
type	step	epoch	samples	loss	lr	skip_pct	sps	elapsed_s
train	100	0	1638400	0.69123	0.001000	0.08	317000	5.2
valid	2000	0	32768000	0.54321	0.000950			95.1
```

typeはtrain（log-intervalごと）またはvalid（valid-intervalごと）。
train行のlossは区間平均。gnuplot・matplotlib・Excelで直接読み込める。

### 実験レジストリ

--registry PATH で学習完了時に1行追記するTSVファイル。

列:

```
timestamp	name	data	data_n	epochs	batch	peak_lr	min_lr	warmup	lambda	best_step	best_valid	final_valid	total_steps	elapsed_s	notes
```

--name NAME と --notes TEXT で識別情報を付与する。ファイルが
存在しなければヘッダ付きで新規作成する。

Elo列は手動追記する（SPRT対局後に結果を書き足す）。

## Consequences

- 学習カーブを外部ツールで可視化できる。過学習の兆候を判断
  しやすくなる
- 実験条件と結果を1ファイルで一覧比較できる。P6のスケーリング
  実験やハイパラ探索の記録が残る
- TSV形式は最小限の仕組みで、追加の依存やDBが不要
- --resume で再開した場合、ログファイルに追記する（上書きしない）
