# 0014: Position構造とdo/undo

- Status: accepted
- Date: 2026-07-17
- 関連ADR: [0002](0002-cargo-workspace.md), [0009](0009-piece-encoding.md), [0010](0010-bitboard-layout.md), [0012](0012-move-encoding.md), [0013](0013-hand-packing.md)

## Context

Positionは局面のすべてを持ち、do_move/undo_moveは探索で最も頻繁に
実行される操作になる。ここでの決定は3つ。局面の巻き戻し方式、
付随情報（StateInfo）の持ち方、そしてNNUE差分計算（P4）の要件を
どう先読みするかである。

先読みが必要な理由は2つある。NNUEのaccumulatorは1手ごとの差分
（動いた駒の増減 = DirtyPiece）から更新するため、do_moveが差分を
記録する構造になっていないとP4で手戻りする。また、coreクレートは
探索・評価に依存しない方針（ADR-0002）のため、NNUE固有の型を
coreに置かずに差分だけを公開する境界設計が要る。

## 選択肢と比較

### 巻き戻し方式

#### 案A: copy-make（ノードごとにPositionを複製）

undoが不要になり実装が単純。ただしPositionは盤面配列81B＋
bitboard群約300B＋付随情報で400Bを超え、ノードごとのmemcpyが
探索全体に効く。NNUEのaccumulator（数KB）を複製に含めると さらに
不利で、含めないなら結局スタック管理が別に要る。

#### 案B: make/unmake＋StateInfoスタック

do_moveで「取った駒・ハッシュ・王手情報」などの巻き戻し材料を
StateInfoに積み、undo_moveでpopして復元する。Stockfish・やねうら王・
OSLすべてこの方式。accumulatorのスタックとも構造が揃う。

### StateInfoの持ち方（Rust表現）

#### 案C: 生ポインタのチェーン（Stockfish流 `st->previous`）

C++の慣行そのまま。Rustではライフタイムを諦めてunsafeになり、
ADR-0004の許容カテゴリに入らない。

#### 案D: PositionがVec<StateInfo>を所有し、添字で辿る

千日手検出の遡りは添字のデクリメントで書ける。Vecの再確保を
避けるため容量を初期化時に確保する。unsafe不要で、Positionの
cloneでスタックごと複製される（Lazy SMPのスレッド分配と相性が良い）。

## Decision

案B＋案Dを採用する。定義は次のとおり。

### Positionのフィールド

```
board:        [Piece; 81]           // マス→駒の逆引き
by_type:      [Bitboard; 16]        // PieceType別（先後混合、OSL配列添字）
by_color:     [Bitboard; 2]         // 先手駒・後手駒の占有
hands:        [Hand; 2]
side_to_move: Color
game_ply:     u16
king_sq:      [Square; 2]           // 玉位置のキャッシュ
states:       Vec<StateInfo>        // 巻き戻しスタック（初期容量1024）
```

個別の駒のbitboardは `by_type[pt] & by_color[c]` で得る。
boardとbitboardは冗長だが、マスからの駒種参照と集合演算の
両方をO(1)にするための標準構成。

### StateInfoのフィールド

```
captured:         Piece             // 取った駒（なければEMPTY）
board_key:        u64               // Zobrist（次のADRで起草）
hand_key:         u64
checkers:         Bitboard          // 手番玉に王手している駒
blockers_for_king:[Bitboard; 2]     // pin候補（両玉分）
pinners:          [Bitboard; 2]
check_squares:    [Bitboard; 16]    // 駒種別の王手可能マス（gives_check用）
continuous_check: [u16; 2]          // 連続王手カウンタ（連続王手の千日手用）
plies_from_null:  u16               // 千日手遡りの上限
material:         i32               // 駒割の差分累計（評価v1用）
dirty:            DirtyPiece        // NNUE差分の材料（下記）
```

### DirtyPiece（NNUE要件の先読み）

1手で状態が変わる駒は最大2枚（動かした駒＋取られた駒）。
玉の移動はaccumulator全再計算になるためフラグで区別する。

```
DirtyPiece {
    count: u8,                      // 変化した駒の数（1..=2）
    king_moved: bool,               // 手番側の玉が動いたか
    piece_old: [Piece; 2],          // 変化前（打ちでは手駒由来のEMPTY扱い）
    piece_new: [Piece; 2],          // 変化後（成りなら成駒、捕獲なら相手手駒へ）
    from:      [Square; 2],         // 移動元（手駒はSquare::NONE）
    to:        [Square; 2],         // 移動先（手駒に入る場合はSquare::NONE）
}
```

BonaPiece番号への変換はnnueクレート側で行い、coreは盤・手駒の
語彙だけで差分を記録する。do_moveは常にDirtyPieceを埋める（契約）。
NNUEのaccumulator本体はcoreに置かず、nnueクレートが探索plyに
沿った自前のスタックを持ち、StateInfoのDirtyPieceを読んで更新する。
これでcoreの探索非依存（ADR-0002）とP4の差分計算が両立する。

### do_move / undo_move

- `do_move(&mut self, m: Move, gives_check: bool)`。王手判定は
  呼び出し側がcheck_squaresで安価に前計算して渡す（Stockfish方式）
- do_moveの中で新しいStateInfoを積み、checkers・blockers・pinners・
  check_squaresを新しい占有状態から再計算する
- `undo_move(&mut self, m: Move)` はStateInfoをpopし、boardとbitboardを
  Moveと`captured`から復元する
- Lazy SMPではスレッドごとにPositionをcloneする。states込みの
  複製で、探索開始時の1回だけなのでコストは無視できる

## Consequences

- do/undo往復の完全一致（board・bitboard・hands・king_sq・各key）を
  property testで担保する（ADR-0006の規約。実装より先にテストを書く）
- DirtyPieceの記録はperftでも常に実行される。数命令のストア増だが、
  分岐で消すより単純さを取る。perftのNPSで許容できないと分かったら
  見直す
- StateInfoは1エントリ約350B（check_squares 16×16Bが大半）。
  深さ128の探索で約45KBに収まる
- check_squaresをStateInfoに置くかは実装時にサイズと再計算コストを
  見て調整してよい（置かない場合は都度計算）。ADRとしては
  「gives_checkがO(1)で判定できること」を要件として固定する
- 巻き戻しに使うcapturedはStateInfo側、移動情報はMove側（ADR-0012）
  という分担が確定する
