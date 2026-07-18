# 0019: USI実装アーキテクチャ

- Status: proposed
- Date: 2026-07-18
- 関連ADR: [0002](0002-cargo-workspace.md), [0018](0018-sfen-perft.md)

## Context

USIプロトコルはstdin/stdoutの行単位テキストで、GUIが主導する。
難しいのは非同期性で、`go` で探索を始めた後も `stop` / `ponderhit` /
`quit` を受け付けて即座に反応する必要がある。探索が回っている間に
stdinを読めるスレッド構造が前提になる。

## 選択肢と比較

### 案A: 単一スレッドの同期ループ

`go` で探索関数を呼ぶと、探索が返るまでstdinを読めない。
`stop` に反応できずUSI違反になる。不採用。

### 案B: stdin読み取りスレッド＋コマンドループ＋探索スレッド分離

stdinは専用スレッドが読み、チャネルでコマンドループへ渡す。
探索は探索スレッド（ADR-0020）で回し、停止はatomicフラグで伝える。
コマンドループは探索中もチャネルを読み続けられる。
Stockfish・やねうら王と同じ構造。

## Decision

案Bを採用する。usiクレートの構成は次のとおり。

- stdinスレッド: `BufRead::lines` で読み、mpscチャネルで
  コマンドループへ送る。EOFは `quit` として扱う
- コマンドループ（メインスレッド）: コマンドをパースして処理する。
  `go` は探索スレッドを起こして即座にループへ戻る
- 探索への割り込み: `stop`/`quit` はAtomicBool（Relaxed）の
  stopフラグ、`ponderhit` は専用フラグで伝える（ADR-0020）
- 出力: bestmove/infoは探索スレッドからも出るため、行単位の
  排他を `Stdout::lock` で行い、毎行flushする
- `position`: sfen（またはstartpos）＋moves列を毎回ゼロから
  再構築する。差分適用の最適化はしない（1局面数μsで足りる）
- `setoption`: 宣言的なオプションレジストリを持つ。型は
  spin / check / string / combo / button。P2で載せるのは
  USI_Hash、USI_Ponder、Threads、NetworkDelay、NetworkDelay2、
  MaxMovesToDraw。`usi` 応答のoption列挙もレジストリから生成する
- `isready`: 置換表確保などの重い初期化はここで行い `readyok` を返す
- `bestmove` は `resign`（合法手なし）と `win`（宣言勝ち、P3で実装）に対応する

## Consequences

- コマンドループと探索の間の共有状態が「停止フラグ類＋探索開始時に
  渡すパラメータ」だけに限定され、データ競合の余地が小さい
- USIゴールデンテスト（コマンド列を流して応答列を比較）が
  プロセス外から書ける。stop割り込みのタイミング依存テストは
  「stop後に必ずbestmoveが返る」ことの検証に絞る
- position再構築のたびにSFEN検証（ADR-0018）が走る。GUI由来の
  不正入力に対して落ちずにエラーを返せる
