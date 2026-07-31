# 0054: qsearchに置換表を導入する

- Status: accepted
- Date: 2026-07-22
- 関連ADR: [0022](0022-transposition-table.md), [0028](0028-pruning-extensions.md), [0049](0049-eval-hash.md)

## Context

探索改善キャンペーンの第8弾。qsearchは現在TTを一切使わない
（probeもstoreもない。`search.rs:872-`）。探索ノードの大半は
qsearchであり、置換の多い将棋では同一局面のqsearch木を
何度も読み直している。eval hash（ADR-0049）は評価1回分しか
省けないが、TTなら静止探索の結論（boundとbest move）ごと
再利用できる。SF系ではqsearchのTT probe/storeは標準装備で、
ROADMAPの候補の「qsearchのTT保存拡充」に当たる。

## 選択肢と比較

### 案A: probe + store + TT手のオーダリング利用

qsearch入口でprobeし、boundが許せば即カット。TT手は
qsearchのMovePickerで最初に試す。出口でdepth 0として
bound付きstoreする。SFと同じ完全形。

### 案B: storeのみ（カットしない）

main searchからの再訪だけが恩恵を受ける形。安全だが、
qsearch同士の再訪（数が最も多い）を活かせず中途半端。

## Decision

案Aを採用する。

### 実装スケッチ（search.rs）

probe（qsearch入口、stand patの前）:
- `pos.key()`でTTをprobeし、ヒットしたら`value_from_tt`で
  ply補正のうえboundを検査。non-PVでは
  lower boundかつ値>=beta、upper boundかつ値<=alpha、
  exactのいずれかで即return（fail-soft）
- TTのeval欄があればstand patの生評価として再利用する
  （eval hashより優先。main searchと同じ扱い）
- TT手はpickerの先頭で試す（qsearch用MovePickerにtt_moveを
  渡す。既存のTtMoveステージの流儀を踏襲）

store（qsearch出口）:
- depth 0、boundはfail-high/low/exactに応じて設定、
  eval欄は生評価（王手中はVALUE_NONE）
- 王手回避で手がない（詰み）の値もそのまま保存してよい
  （value_to_ttでply補正）

初期定数: なし（構造の追加のみ。深さは0固定）。

### 検証

SPRTはADR-0028の既定条件。両エンジンに
`--option "EvalFile=data/halfkp_180M.hmwr.best"`。

## Consequences

- TTへの書き込みがqsearchノード分増え、main searchの深い
  エントリの寿命が統計的に縮む。depth 0エントリは置換で
  すぐ上書きされる側なので影響は限定的とみるが、H0の場合は
  この圧迫が主因の疑いがある（bucket化・置換方針はROADMAPの候補の
  別項目）
- qsearch入口のTT probeはメモリアクセス1回のコスト。カット
  率がこれを上回るかはSPRTで判定する
- 入口ply（qdepth=0）の静かな王手生成との順序は既存のまま。
  TT手が王手だった場合も正しく先頭で試される
