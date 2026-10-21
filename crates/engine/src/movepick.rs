//! 指し手オーダリングとhistoryのテーブル群（ADR-0025, 0109）。
//!
//! MovePickerは段階生成の状態機械。参照実装（やねうら王）の
//! `movepick.cpp` の遷移をそのまま移してある。カットが早いノードでは
//! Quietsの生成自体を省く。borrow衝突を避けるため、next()は毎回
//! &Positionと&Historiesを受け取る。
//!
//! テーブルの次元・値域・初期値は参照実装の `history.h` と
//! `yaneuraou-search.cpp` を出典とする（ADR-0074）。

use std::sync::Arc;
use std::sync::atomic::{AtomicI16, Ordering};

use himawari_core::{
    Bitboard, Color, GenType, Move, MoveList, Piece, PieceType, Square, piece_value,
};

use crate::board::SearchBoard;

// ---- テーブルの寸法 ----

/// 駒の種類数（先後込み）。参照実装のPIECE_NB。
const PIECE_NB: usize = 32;
/// マスの数。参照実装のSQUARE_NB。
const SQUARE_NB: usize = 81;
/// 駒種の数（先後なし）。参照実装のPIECE_TYPE_NB。
const PIECE_TYPE_NB: usize = 16;
/// 16bitの指し手をそのまま添字にする表のサイズ（history.h:27）。
const UINT16_HISTORY_SIZE: usize = 65536;
/// lowPly historyが覆うply数（history.h:31）。
pub const LOW_PLY_HISTORY_SIZE: usize = 5;
/// 歩構造historyのスロット数（history.h:26）。
const PAWN_HISTORY_BASE_SIZE: usize = 8192;
/// correction historyのスロット数（history.h:29）。
const CORRHIST_BASE_SIZE: usize = UINT16_HISTORY_SIZE;

// ---- 値域（StatsEntryのD） ----

/// main history / lowPly historyの値域（history.h:206, 239）。
const D_BUTTERFLY: i32 = 7183;
/// capture historyの値域（history.h:244）。
const D_CAPTURE: i32 = 10692;
/// continuation historyの値域（history.h:249のPieceToHistory）。
const D_PIECE_TO: i32 = 30000;
/// pawn historyの値域（history.h:266）。
const D_PAWN: i32 = 8192;
/// correction historyの値域（history.h:30）。
const D_CORRECTION: i32 = 1024;
/// ttMoveHistoryの値域（history.h:345）。
const D_TT_MOVE: i32 = 8192;

// ---- 初期値（YaneuraOuWorker::clear。yaneuraou-search.cpp:2139-2176） ----

/// main historyの初期値（yaneuraou-search.cpp:703, 2142）。
const INIT_MAIN: i16 = 0;
/// capture historyの初期値（yaneuraou-search.cpp:2143）。
const INIT_CAPTURE: i16 = -678;
/// pawn historyの初期値（yaneuraou-search.cpp:2147）。
const INIT_PAWN: i16 = -1238;
/// continuation correction historyの初期値（yaneuraou-search.cpp:2153）。
const INIT_CONT_CORR: i16 = 6;
/// continuation historyの初期値（yaneuraou-search.cpp:2165）。
const INIT_CONT: i16 = -523;
/// lowPly historyの初期値。goのたびに埋め直す（yaneuraou-search.cpp:1540）。
const INIT_LOW_PLY: i16 = 98;

/// gravity方式の更新（history.h:91-125のStatsEntry::operator<<）。
/// bonusを±dに丸めたうえで、値が±dを超えないよう自然にゼロへ引き戻す。
#[inline]
fn stats_update(entry: &mut i16, bonus: i32, d: i32) {
    let b = bonus.clamp(-d, d);
    let v = i32::from(*entry);
    *entry = (v + b - v * b.abs() / d) as i16;
}

/// スレッド共有の表に対する同じ更新（ADR-0162。history.hの `AtomicStats`）。
/// 読んで書く間に別スレッドが書くと片方の更新が消える。参照実装も同じ
/// 扱いで、historyは統計値なので取りこぼしを許容する。
#[inline]
fn stats_update_atomic(entry: &AtomicI16, bonus: i32, d: i32) {
    let b = bonus.clamp(-d, d);
    let v = i32::from(entry.load(Ordering::Relaxed));
    entry.store((v + b - v * b.abs() / d) as i16, Ordering::Relaxed);
}

/// 初期値で埋めた原子配列を作る。
fn atomic_table(len: usize, init: i16) -> Box<[AtomicI16]> {
    (0..len).map(|_| AtomicI16::new(init)).collect()
}

/// main history（ButterflyHistory。[手番 2][指し手16bit 65536]。history.h:206）。
///
/// 参照実装と同じく指し手の生16bitを添字にする。移動元を区別するので、
/// 同じ駒が同じマスへ入る手でも来た筋ごとに別の値を持つ。
/// 本エンジンの16bitは `to(7) | from(7) | 成 | 打` で、駒打ちのfrom欄には
/// 駒種（9〜15）が入る。盤上のマス9〜15と数値は重なるが、打ちビットが
/// 立つので16bit全体では衝突しない（history.h:217-229の方式に対応する）。
pub struct History {
    table: Box<[i16]>,
}

impl Default for History {
    fn default() -> Self {
        History {
            table: vec![INIT_MAIN; 2 * UINT16_HISTORY_SIZE].into_boxed_slice(),
        }
    }
}

impl History {
    #[inline]
    fn index(c: Color, m: Move) -> usize {
        c.index() * UINT16_HISTORY_SIZE + usize::from(m.to_move16().0)
    }

    #[inline]
    pub fn get(&self, c: Color, m: Move) -> i32 {
        i32::from(self.table[Self::index(c, m)])
    }

    pub fn update(&mut self, c: Color, m: Move, bonus: i32) {
        stats_update(&mut self.table[Self::index(c, m)], bonus, D_BUTTERFLY);
    }

    pub fn clear(&mut self) {
        self.table.fill(INIT_MAIN);
    }
}

/// lowPly history（[ply 5][指し手16bit]。history.h:239）。
/// root付近のオーダリングを整える。goのたびに98で埋め直す。
pub struct LowPlyHistory {
    table: Box<[i16]>,
}

impl Default for LowPlyHistory {
    fn default() -> Self {
        LowPlyHistory {
            table: vec![INIT_LOW_PLY; LOW_PLY_HISTORY_SIZE * UINT16_HISTORY_SIZE]
                .into_boxed_slice(),
        }
    }
}

impl LowPlyHistory {
    #[inline]
    fn index(ply: usize, m: Move) -> usize {
        ply * UINT16_HISTORY_SIZE + usize::from(m.to_move16().0)
    }

    #[inline]
    pub fn get(&self, ply: usize, m: Move) -> i32 {
        i32::from(self.table[Self::index(ply, m)])
    }

    pub fn update(&mut self, ply: usize, m: Move, bonus: i32) {
        stats_update(&mut self.table[Self::index(ply, m)], bonus, D_BUTTERFLY);
    }

    /// goのたびに98で埋め直す（yaneuraou-search.cpp:1540）。
    pub fn fill_for_new_search(&mut self) {
        self.table.fill(INIT_LOW_PLY);
    }
}

/// capture history（[移動後の駒 32][移動先 81][取った駒の種類 16]。history.h:244）。
pub struct CaptureHistory {
    table: Box<[i16]>,
}

impl Default for CaptureHistory {
    fn default() -> Self {
        CaptureHistory {
            table: vec![INIT_CAPTURE; PIECE_NB * SQUARE_NB * PIECE_TYPE_NB].into_boxed_slice(),
        }
    }
}

impl CaptureHistory {
    #[inline]
    fn index(pc: Piece, to: Square, captured: PieceType) -> usize {
        (pc.index() * SQUARE_NB + to.index()) * PIECE_TYPE_NB + captured.index()
    }

    #[inline]
    pub fn get(&self, pc: Piece, to: Square, captured: PieceType) -> i32 {
        i32::from(self.table[Self::index(pc, to, captured)])
    }

    pub fn update(&mut self, pc: Piece, to: Square, captured: PieceType, bonus: i32) {
        stats_update(
            &mut self.table[Self::index(pc, to, captured)],
            bonus,
            D_CAPTURE,
        );
    }

    pub fn clear(&mut self) {
        self.table.fill(INIT_CAPTURE);
    }
}

/// pawn history（[歩構造キー 8192×スレッド数][移動後の駒 32][移動先 81]。
/// history.h:265）。全スレッドで共有する（ADR-0162）。スロット数は
/// スレッド数に比例して伸びる（`DynStats`。history.h:151-153）。
pub struct PawnHistory {
    table: Box<[AtomicI16]>,
    /// スロットのマスク（スロット数−1）。
    mask: usize,
}

impl PawnHistory {
    fn new(slots: usize) -> Self {
        PawnHistory {
            table: atomic_table(slots * PIECE_NB * SQUARE_NB, INIT_PAWN),
            mask: slots - 1,
        }
    }

    /// 歩構造キーからスロットを引く（history.h:370-372）。
    #[inline]
    pub fn slot(&self, pawn_key: u64) -> usize {
        (pawn_key as usize & self.mask) * PIECE_NB * SQUARE_NB
    }

    #[inline]
    pub fn get(&self, slot: usize, pc: Piece, to: Square) -> i32 {
        i32::from(self.table[slot + pc.index() * SQUARE_NB + to.index()].load(Ordering::Relaxed))
    }

    pub fn update(&self, slot: usize, pc: Piece, to: Square, bonus: i32) {
        stats_update_atomic(
            &self.table[slot + pc.index() * SQUARE_NB + to.index()],
            bonus,
            D_PAWN,
        );
    }

    pub fn clear(&self) {
        for e in &self.table {
            e.store(INIT_PAWN, Ordering::Relaxed);
        }
    }
}

/// 静的評価の系統誤差を補正する履歴（ADR-0046, 0109）。
///
/// 参照実装のUnifiedCorrectionHistory（history.h:337-339）に対応する。
/// 1本の表を4系統（歩・小駒・先手非歩・後手非歩）で共有し、系統ごとに
/// 別のキーで引く。添字は `[キー下位16bit×スレッド数][手番][系統]`。
/// 全スレッドで共有し、スロット数はスレッド数に比例して伸びる
/// （ADR-0162。history.h:151-153, 359-366）。
pub struct CorrectionHistory {
    table: Box<[AtomicI16]>,
    /// スロットのマスク（スロット数−1）。
    mask: usize,
}

/// correction historyの系統（history.h:296-309のCorrectionBundle）。
const CORR_PAWN: usize = 0;
const CORR_MINOR: usize = 1;
const CORR_NON_PAWN_BLACK: usize = 2;
const CORR_NON_PAWN_WHITE: usize = 3;
/// 1スロットが持つ系統数。
const CORR_KINDS: usize = 4;

impl CorrectionHistory {
    fn new(slots: usize) -> Self {
        CorrectionHistory {
            table: atomic_table(slots * 2 * CORR_KINDS, 0),
            mask: slots - 1,
        }
    }

    #[inline]
    fn index(&self, key: u64, stm: usize, kind: usize) -> usize {
        ((key as usize & self.mask) * 2 + stm) * CORR_KINDS + kind
    }

    #[inline]
    fn get(&self, key: u64, stm: usize, kind: usize) -> i32 {
        i32::from(self.table[self.index(key, stm, kind)].load(Ordering::Relaxed))
    }

    fn update(&self, key: u64, stm: usize, kind: usize, bonus: i32) {
        stats_update_atomic(&self.table[self.index(key, stm, kind)], bonus, D_CORRECTION);
    }

    /// 4系統の合成前の生の値を取り出す（yaneuraou-search.cpp:728-731）。
    /// 返り値は (歩, 小駒, 先手非歩, 後手非歩)。
    #[inline]
    pub fn probe(&self, pos: &SearchBoard) -> (i32, i32, i32, i32) {
        let stm = pos.side_to_move().index();
        (
            self.get(pos.pawn_key(), stm, CORR_PAWN),
            self.get(pos.minor_piece_key(), stm, CORR_MINOR),
            self.get(
                pos.non_pawn_key(himawari_core::Color::Black),
                stm,
                CORR_NON_PAWN_BLACK,
            ),
            self.get(
                pos.non_pawn_key(himawari_core::Color::White),
                stm,
                CORR_NON_PAWN_WHITE,
            ),
        )
    }

    /// 4系統をまとめて更新する（yaneuraou-search.cpp:759-762）。
    /// 系統ごとの重みも参照実装のものを使う。
    pub fn update_all(&self, pos: &SearchBoard, bonus: i32) {
        /// 非歩系統の重み（yaneuraou-search.cpp:755）。
        const NON_PAWN_WEIGHT: i32 = 187;
        let stm = pos.side_to_move().index();
        self.update(pos.pawn_key(), stm, CORR_PAWN, bonus);
        self.update(pos.minor_piece_key(), stm, CORR_MINOR, bonus * 153 / 128);
        self.update(
            pos.non_pawn_key(himawari_core::Color::Black),
            stm,
            CORR_NON_PAWN_BLACK,
            bonus * NON_PAWN_WEIGHT / 128,
        );
        self.update(
            pos.non_pawn_key(himawari_core::Color::White),
            stm,
            CORR_NON_PAWN_WHITE,
            bonus * NON_PAWN_WEIGHT / 128,
        );
    }

    pub fn clear(&self) {
        for e in &self.table {
            e.store(0, Ordering::Relaxed);
        }
    }
}

/// 全スレッドで共有するhistory（ADR-0162。`history.h:359` の
/// `SharedHistories`）。参照実装が共有するのはcorrection historyと
/// pawn historyの2面だけで、continuation correction historyは
/// スレッドローカルに残る（yaneuraou-search.h:457）。
pub struct SharedHistories {
    pub pawn: PawnHistory,
    pub corr: CorrectionHistory,
}

impl SharedHistories {
    /// スレッド数に比例したスロット数で確保する（history.h:151-153）。
    /// 参照実装はスレッド数が2の冪であることを要求する（history.h:363）。
    /// 本エンジンは任意の `Threads` を受けるので、2の冪へ切り上げる。
    pub fn new(threads: usize) -> Self {
        let mult = threads.max(1).next_power_of_two();
        SharedHistories {
            pawn: PawnHistory::new(PAWN_HISTORY_BASE_SIZE * mult),
            corr: CorrectionHistory::new(CORRHIST_BASE_SIZE * mult),
        }
    }

    /// 対局間のリセット（yaneuraou-search.cpp:2139-2176）。
    pub fn clear(&self) {
        self.pawn.clear();
        self.corr.clear();
    }
}

impl Default for SharedHistories {
    fn default() -> Self {
        SharedHistories::new(1)
    }
}

/// continuation correction history（ADR-0085, 0109）。
/// 論理次元は[条件手の駒 32][条件手の移動先 81][駒 32][マス 81]
/// （history.h:325-327）。条件手の側は2手前・4手前を見る。
pub struct ContinuationCorrectionHistory {
    table: Box<[i16]>,
}

/// 応手側の面の広さ（[駒][マス]）。
const CONT_STRIDE: usize = PIECE_NB * SQUARE_NB;

impl Default for ContinuationCorrectionHistory {
    fn default() -> Self {
        ContinuationCorrectionHistory {
            table: vec![INIT_CONT_CORR; CONT_STRIDE * CONT_STRIDE].into_boxed_slice(),
        }
    }
}

impl ContinuationCorrectionHistory {
    /// 条件手から面の先頭添字を作る。指し手がないplyは番兵の面（先頭）を指す
    /// （yaneuraou-search.cpp:1469, 3256）。
    #[inline]
    pub fn base(m: Move) -> usize {
        if m.is_special() {
            return 0;
        }
        (m.piece_after().index() * SQUARE_NB + m.to().index()) * CONT_STRIDE
    }

    #[inline]
    pub fn get(&self, base: usize, pc: Piece, to: Square) -> i32 {
        i32::from(self.table[base + pc.index() * SQUARE_NB + to.index()])
    }

    pub fn update(&mut self, base: usize, pc: Piece, to: Square, bonus: i32) {
        stats_update(
            &mut self.table[base + pc.index() * SQUARE_NB + to.index()],
            bonus,
            D_CORRECTION,
        );
    }

    pub fn clear(&mut self) {
        self.table.fill(INIT_CONT_CORR);
    }
}

/// continuation history（ADR-0047, 0109）。
/// 論理次元は[王手中か 2][駒を取る手か 2][条件手の駒 32][条件手の移動先 81]
/// [応手の駒 32][応手の移動先 81]（history.h:259、yaneuraou-search.cpp:2117）。
/// 巨大ネスト配列のスタック経由初期化はオーバーフローの危険があるため、
/// フラットなboxed sliceで確保し添字を計算する（約51.3MiB）。
pub struct ContinuationHistory {
    table: Box<[i16]>,
}

impl Default for ContinuationHistory {
    fn default() -> Self {
        ContinuationHistory {
            table: vec![INIT_CONT; 4 * CONT_STRIDE * CONT_STRIDE].into_boxed_slice(),
        }
    }
}

impl ContinuationHistory {
    /// 条件手から面の先頭添字を作る（yaneuraou-search.cpp:2117）。
    /// in_checkはその手を指したノードで王手がかかっていたか、
    /// captureはその手が駒を取る手か。
    #[inline]
    pub fn base(in_check: bool, capture: bool, pc: Piece, to: Square) -> usize {
        (((usize::from(in_check) * 2 + usize::from(capture)) * PIECE_NB + pc.index()) * SQUARE_NB
            + to.index())
            * CONT_STRIDE
    }

    /// 指し手のないplyが指す番兵の面（yaneuraou-search.cpp:1467, 3255）。
    pub const SENTINEL: usize = 0;

    #[inline]
    pub fn get(&self, base: usize, pc: Piece, to: Square) -> i32 {
        i32::from(self.table[base + pc.index() * SQUARE_NB + to.index()])
    }

    pub fn update(&mut self, base: usize, pc: Piece, to: Square, bonus: i32) {
        stats_update(
            &mut self.table[base + pc.index() * SQUARE_NB + to.index()],
            bonus,
            D_PIECE_TO,
        );
    }

    pub fn clear(&mut self) {
        self.table.fill(INIT_CONT);
    }
}

/// TT手が最善手になりやすいかを1個のスカラーで持つ（history.h:345）。
#[derive(Default)]
pub struct TtMoveHistory {
    value: i16,
}

impl TtMoveHistory {
    #[inline]
    pub fn get(&self) -> i32 {
        i32::from(self.value)
    }

    pub fn update(&mut self, bonus: i32) {
        stats_update(&mut self.value, bonus, D_TT_MOVE);
    }

    pub fn clear(&mut self) {
        self.value = 0;
    }
}

/// スレッドが対局を通じて保持するhistoryの一式（ADR-0109）。
/// 参照実装のWorkerが持つテーブル群に対応する。
#[derive(Default)]
pub struct Histories {
    pub main: History,
    pub low_ply: LowPlyHistory,
    pub capture: CaptureHistory,
    pub cont: ContinuationHistory,
    pub corr_cont: ContinuationCorrectionHistory,
    pub tt_move: TtMoveHistory,
    /// 全スレッドで共有する2面（ADR-0162）。pawn historyと
    /// correction historyはここから引く
    pub shared: Arc<SharedHistories>,
}

impl Histories {
    pub fn new(shared: Arc<SharedHistories>) -> Self {
        Histories {
            shared,
            ..Default::default()
        }
    }

    /// 対局間のリセット（yaneuraou-search.cpp:2139-2176）。
    /// 共有分は呼び出し側が1度だけ消す（全スレッドで同じ表なので）。
    pub fn clear(&mut self) {
        self.main.clear();
        self.capture.clear();
        self.cont.clear();
        self.corr_cont.clear();
        self.tt_move.clear();
    }

    /// goごとの初期化。lowPly historyだけは局面ごとに埋め直す
    /// （yaneuraou-search.cpp:1539-1540）。
    pub fn new_search(&mut self) {
        self.low_ply.fill_for_new_search();
    }
}

/// 段階生成の状態（movepick.cpp:22-63）。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    MainTt,
    CaptureInit,
    GoodCapture,
    QuietInit,
    GoodQuiet,
    BadCapture,
    BadQuiet,
    EvasionTt,
    EvasionInit,
    Evasion,
    QsearchTt,
    QCaptureInit,
    QCapture,
    ProbCutTt,
    ProbCutInit,
    ProbCut,
    Done,
}

/// 採点済みの指し手（参照実装のExtMove）。
///
/// 手と点数を、並べ替えの鍵になる1語に詰めて持つ（ADR-0243）。上位32ビットが点数
/// （符号ビットを反転して、符号なしの比較で点数の順になる形）、下位32ビットが手の符号の否定である。
/// 鍵が大きいほど先に返す。点数の降順に並び、同点なら手の符号の昇順に並ぶ。
/// 順序が全順序なので、並びは生成の順に依存しない
#[derive(Clone, Copy)]
struct ExtMove {
    k: u64,
}

impl ExtMove {
    #[inline(always)]
    fn new(m: Move, v: i32) -> ExtMove {
        ExtMove {
            k: (u64::from(v as u32 ^ 0x8000_0000) << 32) | u64::from(!m.bits()),
        }
    }

    #[inline(always)]
    fn m(&self) -> Move {
        Move::from_bits(!(self.k as u32))
    }

    #[inline(always)]
    fn v(&self) -> i32 {
        ((self.k >> 32) as u32 ^ 0x8000_0000) as i32
    }
}

/// ノード1つで生成される手を見込んだ初期容量。
const MOVE_BUF_CAP: usize = 64;

/// MovePickerへ貸し出す採点済みの手の置き場（ADR-0151の群B）。
///
/// `ExtMove` をモジュールの外へ出さずに、`Worker` がply別の領域を
/// 持ち回るための入れ物である。`Default` は空を返すので、
/// `std::mem::take` で貸し出しても確保が起きない。
#[derive(Default)]
pub struct MoveBuf(Vec<ExtMove>);

impl MoveBuf {
    /// ply別バッファの初期確保に使う。以後は返却のたびに容量が残るので、
    /// そのplyで生成された最大手数まで伸びたあとは再確保が起きない。
    pub fn with_node_capacity() -> MoveBuf {
        MoveBuf(Vec::with_capacity(MOVE_BUF_CAP))
    }
}

/// 容量を確保した後の積み込み。容量の検査と伸ばす経路を手ごとに通らない（ADR-0241の第16回）。
#[inline(always)]
fn push_reserved(v: &mut Vec<ExtMove>, e: ExtMove) {
    debug_assert!(v.len() < v.capacity());
    // SAFETY: 点数付けの前に `reserve(list.len())` で、積む手の数だけ容量を確保してある。
    // 積むのは手の列の各手について1回だけなので、長さは容量を超えない
    unsafe {
        let len = v.len();
        v.as_mut_ptr().add(len).write(e);
        v.set_len(len + 1);
    }
}

/// 良い静かな手とみなすスコアの下限（movepick.cpp:459）。
const GOOD_QUIET_THRESHOLD: i32 = -14000;
/// 静かな手の部分ソートの閾値係数（movepick.cpp:605）。
const QUIET_SORT_COEF: i32 = -3560;
/// 静かな王手へのボーナスと、その資格を測るSEEの下限（movepick.cpp:371）。
const CHECK_BONUS: i32 = 16384;
const CHECK_SEE_MARGIN: i32 = -75;

/// 鍵の順（点数の降順・同点なら手の符号の昇順）に挿入ソートで並べる（ADR-0243）。
/// 取る手・王手回避と、静かな手の閾値以上の手に使う。どれも数が少ない。
///
/// 添字は範囲検査なしで引く（ADR-0241の第16回）。i は `1..len`、j は i 以下で、
/// j-1 を読むのは j≠0 のときだけなので、どれも len 未満である。
fn insertion_sort_key(a: &mut [ExtMove]) {
    let len = a.len();
    let p = a.as_mut_ptr();
    // SAFETY: 上の添字の範囲の説明のとおり、読み書きする添字はすべて len 未満である
    unsafe {
        for i in 1..len {
            let t = *p.add(i);
            let mut j = i;
            while j > 0 && (*p.add(j - 1)).k < t.k {
                *p.add(j) = *p.add(j - 1);
                j -= 1;
            }
            *p.add(j) = t;
        }
    }
}

/// 条件を満たす手を先頭へ寄せ、その数を返す。寄せた範囲と残りの範囲の中の順は問わない（ADR-0243）。
///
/// 分岐を使わずに書く。毎回入れ替え、条件を満たしたときだけ境界を進める。条件の真偽は手ごとに読めないので、
/// 分岐で書くと外れが多い。分岐で書いた形は命令数が少なかったのに、時間では重かった
#[inline(always)]
fn partition(a: &mut [ExtMove], f: impl Fn(&ExtMove) -> bool) -> usize {
    let len = a.len();
    let p = a.as_mut_ptr();
    let mut n = 0;
    // SAFETY: i は len 未満、n は i 以下なので、どちらも範囲内である
    unsafe {
        for i in 0..len {
            let x = *p.add(i);
            *p.add(i) = *p.add(n);
            *p.add(n) = x;
            n += usize::from(f(&x));
        }
    }
    n
}

/// 静かな手の並んでいない範囲を、一度に並べる手の数。
/// 返す手が少ないノードで全部を並べないよう、先頭から少しずつ選んで並べる。
const EXTEND_CHUNK: usize = 8;

/// 範囲の上位 `EXTEND_CHUNK` 手を鍵の順に先頭へ並べ、並べた数を返す（ADR-0243）。
///
/// 先頭の `EXTEND_CHUNK` 手を並べてから残りを1回なめ、並べた最後の手より鍵が大きい手が来たら
/// 入れ替えて差し込む。後ろへ行くほど差し込みは減るので、比べる分岐はほぼ外れない。
fn select_top(a: &mut [ExtMove]) -> usize {
    const K: usize = EXTEND_CHUNK;
    let len = a.len();
    if len <= K {
        insertion_sort_key(a);
        return len;
    }
    insertion_sort_key(&mut a[..K]);
    let p = a.as_mut_ptr();
    // SAFETY: i は `K..len`、j は `1..K` なので、読み書きする添字はすべて len 未満である
    unsafe {
        let mut last = (*p.add(K - 1)).k;
        for i in K..len {
            let x = *p.add(i);
            if x.k > last {
                *p.add(i) = *p.add(K - 1);
                let mut j = K - 1;
                while j > 0 && (*p.add(j - 1)).k < x.k {
                    *p.add(j) = *p.add(j - 1);
                    j -= 1;
                }
                *p.add(j) = x;
                last = (*p.add(K - 1)).k;
            }
        }
    }
    K
}

pub struct MovePicker {
    stage: Stage,
    tt_move: Move,
    /// 静かな手の部分ソートの閾値に使う（movepick.cpp:605）。
    depth: i32,
    /// lowPly historyを引くためのroot からの手数。
    ply: usize,
    /// 静かな手をもう返さない（movepick.cpp:697）。
    skip_quiets: bool,
    /// 採点済みの手。[0, end_captures)が取る手、
    /// [end_captures, end_generated)が静かな手（movepick.cpp:528-563）
    moves: Vec<ExtMove>,
    cur: usize,
    end_cur: usize,
    end_bad_captures: usize,
    end_captures: usize,
    end_generated: usize,
    /// ProbCut用。SEEがこの値以上の取る手だけを返す（movepick.cpp:684）。
    threshold: i32,
    /// 静かな手のうち、鍵の順に並べ終えた範囲の終わり（ADR-0243）。ここから `end_generated` までは、
    /// `select` が読む前に `extend_sorted` で並べる。取る手と王手回避は全部を先に並べる
    sorted_to: usize,
    /// 静かな手のうち、良い静かな手の候補の範囲の終わり（ADR-0243）。この先は悪い静かな手だけである。
    /// 良い静かな手の段が悪い手に当たったら、その位置へ縮める
    end_good_quiets: usize,
}

impl MovePicker {
    fn make(
        stage: Stage,
        tt_move: Move,
        depth: i32,
        ply: usize,
        threshold: i32,
        buf: MoveBuf,
    ) -> Self {
        // 貸し出されたバッファは前のノードの中身が残っている。
        // 生成の段（CaptureInit等）が必ずclearするが、ここでも空にしておく
        let mut moves = buf.0;
        moves.clear();
        MovePicker {
            stage,
            tt_move,
            depth,
            ply,
            skip_quiets: false,
            moves,
            cur: 0,
            end_cur: 0,
            end_bad_captures: 0,
            end_captures: 0,
            end_generated: 0,
            threshold,
            sorted_to: usize::MAX,
            end_good_quiets: 0,
        }
    }

    /// 通常探索・静止探索用（movepick.cpp:120-202）。depth <= 0で静止探索。
    pub fn new(pos: &SearchBoard, tt_move: Move, depth: i32, ply: usize, buf: MoveBuf) -> Self {
        let tt_ok = tt_move != Move::NONE && pos.pseudo_legal(tt_move);
        let stage = if pos.in_check() {
            if tt_ok {
                Stage::EvasionTt
            } else {
                Stage::EvasionInit
            }
        } else if depth > 0 {
            if tt_ok {
                Stage::MainTt
            } else {
                Stage::CaptureInit
            }
        } else if tt_ok {
            Stage::QsearchTt
        } else {
            Stage::QCaptureInit
        };
        Self::make(stage, tt_move, depth, ply, 0, buf)
    }

    /// ProbCut用（movepick.cpp:204-252）。SEEが閾値以上の取る手だけを返す。
    /// 置換表の手は取る手でありさえすれば、SEEを見ずに先に返す
    /// （movepick.cpp:245-248）。ProbCutは王手中に呼ばれない
    pub fn new_probcut(pos: &SearchBoard, tt_move: Move, threshold: i32, buf: MoveBuf) -> Self {
        debug_assert!(!pos.in_check());
        let tt_ok = tt_move != Move::NONE
            && !tt_move.is_drop()
            && !pos.piece_on(tt_move.to()).is_empty()
            && pos.pseudo_legal(tt_move);
        let stage = if tt_ok {
            Stage::ProbCutTt
        } else {
            Stage::ProbCutInit
        };
        Self::make(stage, tt_move, 0, 0, threshold, buf)
    }

    /// バッファを呼び出し側へ返す（ADR-0151の群B）。容量が残るので、
    /// 次に同じplyで借りるときは確保が起きない。
    pub fn into_buf(self) -> MoveBuf {
        MoveBuf(self.moves)
    }

    /// 静かな手をもう返さないよう伝える（movepick.cpp:697）。
    pub fn skip_quiet_moves(&mut self) {
        self.skip_quiets = true;
    }

    /// 条件を満たす次の手を返す（movepick.cpp:436-445）。TT手は返さない。
    #[inline]
    fn select(&mut self, pred: impl Fn(&ExtMove) -> bool) -> Option<Move> {
        debug_assert!(self.end_cur <= self.moves.len());
        while self.cur < self.end_cur {
            if self.cur >= self.sorted_to {
                self.extend_sorted();
            }
            // SAFETY: end_cur は積んだ手の数（moves.len()）以下に保たれる。cur < end_cur なので範囲内
            let e = unsafe { *self.moves.get_unchecked(self.cur) };
            self.cur += 1;
            if e.m() != self.tt_move && pred(&e) {
                return Some(e.m());
            }
        }
        None
    }

    /// 良い静かな手を次に1手返す。
    ///
    /// 静かな手は点数の降順に並ぶので、悪い静かな手に当たったら残りもすべて悪い静かな手である。
    /// そこで止めて、その位置を悪い静かな手の始まりとして覚える（ADR-0243）。返す手の順は、
    /// 全部を見て良い手だけを拾う形と変わらない。
    #[inline]
    fn select_good_quiet(&mut self) -> Option<Move> {
        while self.cur < self.end_good_quiets {
            if self.cur >= self.sorted_to {
                self.extend_sorted();
            }
            let e = self.moves[self.cur];
            if e.v() <= GOOD_QUIET_THRESHOLD {
                self.end_good_quiets = self.cur;
                return None;
            }
            self.cur += 1;
            if e.m() != self.tt_move {
                return Some(e.m());
            }
        }
        None
    }

    /// 静かな手の並んでいない範囲の先頭から、鍵の順で最大 `EXTEND_CHUNK` 手を選んで並べる（ADR-0243）。
    ///
    /// 良い静かな手の候補と悪い静かな手は、`init_quiets` が別の範囲へ分けてある。
    /// 範囲をまたいで選ばないので、良い静かな手の段は悪い静かな手を読まない。
    #[inline(never)]
    fn extend_sorted(&mut self) {
        let cur = self.cur;
        let end = if cur < self.end_good_quiets {
            self.end_good_quiets
        } else {
            self.end_generated
        };
        debug_assert!(cur < end && end <= self.moves.len());
        self.sorted_to = cur + select_top(&mut self.moves[cur..end]);
    }

    /// 取る手のスコア（movepick.cpp:341-342）。
    /// 取った駒の価値を7倍し、capture historyを足す。倍率7は、あとで
    /// `see_ge(m, -value/18)` に渡すためにSEEのスケールへ合わせたもの。
    fn score_captures(&mut self, pos: &SearchBoard, h: &Histories, list: &MoveList) {
        self.moves.reserve(list.len());
        for &m in list {
            let pc = m.piece_after();
            let to = m.to();
            let captured = pos.piece_on(to).piece_type();
            let v = h.capture.get(pc, to, captured) + 7 * piece_value(captured);
            push_reserved(&mut self.moves, ExtMove::new(m, v));
        }
    }

    /// 静かな手のスコア（movepick.cpp:362-393）。
    fn score_quiets(
        &mut self,
        pos: &SearchBoard,
        h: &Histories,
        cont: &[usize; 6],
        list: &MoveList,
    ) {
        self.moves.reserve(list.len());
        let pawn_slot = h.shared.pawn.slot(pos.pawn_key());
        let us = pos.side_to_move();
        // 王手になる升を駒種ごとに1回だけ引く。配列盤の盤は呼ぶたびに作り直すので、
        // 手ごとに引くと重い（ADR-0241の第15回）
        let mut checks: [Option<Bitboard>; 16] = [None; 16];
        for &m in list {
            let pc = m.piece_after();
            let to = m.to();
            let mut v = 2 * h.main.get(us, m);
            v += 2 * h.shared.pawn.get(pawn_slot, pc, to);
            v += h.cont.get(cont[0], pc, to);
            v += h.cont.get(cont[1], pc, to);
            v += h.cont.get(cont[2], pc, to);
            v += h.cont.get(cont[3], pc, to);
            v += h.cont.get(cont[5], pc, to);
            // 王手になる手へのボーナス。直接王手になるマスは
            // Positionのノード内キャッシュが駒種ごとに1回だけ引く
            // （ADR-0151群O）
            let pt = pc.piece_type();
            let check =
                *checks[usize::from(pt.0) & 15].get_or_insert_with(|| pos.check_squares(pt));
            if check.test(to) && pos.see_ge(m, CHECK_SEE_MARGIN) {
                v += CHECK_BONUS;
            }
            if self.ply < LOW_PLY_HISTORY_SIZE {
                v += 8 * h.low_ply.get(self.ply, m) / (1 + self.ply as i32);
            }
            push_reserved(&mut self.moves, ExtMove::new(m, v));
        }
    }

    /// 王手回避のスコア（movepick.cpp:396-421）。
    fn score_evasions(
        &mut self,
        pos: &SearchBoard,
        h: &Histories,
        cont: &[usize; 6],
        list: &MoveList,
    ) {
        self.moves.reserve(list.len());
        let us = pos.side_to_move();
        for &m in list {
            let pc = m.piece_after();
            let to = m.to();
            let captured = pos.piece_on(to);
            let v = if !m.is_drop() && !captured.is_empty() {
                // 取る手が常に上に来るよう下駄を履かせる
                piece_value(captured.piece_type()) + (1 << 28)
            } else {
                h.main.get(us, m) + h.cont.get(cont[0], pc, to)
            };
            push_reserved(&mut self.moves, ExtMove::new(m, v));
        }
    }

    /// 取る手を生成して並べる（movepick.cpp:466-478）。
    ///
    /// **`next` から切り出してあるのは、`MoveList`（608手ぶん、約2.4KB）を
    /// 呼び出しごとのフレームから追い出すためである（ADR-0166）。** 生成段を
    /// 通るのは `next` の呼び出し5〜6回に1回だが、インライン化すると
    /// フレームの確保とレジスタの退避が毎回走る。
    #[inline(never)]
    fn init_captures(&mut self, pos: &SearchBoard, h: &Histories) {
        let mut list = MoveList::default();
        pos.generate(GenType::Captures, false, &mut list);
        self.moves.clear();
        self.score_captures(pos, h, &list);
        self.cur = 0;
        self.end_bad_captures = 0;
        self.end_captures = self.moves.len();
        self.end_generated = self.end_captures;
        self.end_cur = self.end_captures;
        // 取る手は数が多くないので全数ソートでよい
        insertion_sort_key(&mut self.moves);
        self.sorted_to = usize::MAX;
    }

    /// 静かな手を生成して並べる（movepick.cpp:519-533）。
    /// 切り出す理由は `init_captures` と同じ。
    #[inline(never)]
    fn init_quiets(&mut self, pos: &SearchBoard, h: &Histories, cont: &[usize; 6]) {
        let mut list = MoveList::default();
        pos.generate(GenType::Quiets, false, &mut list);
        self.score_quiets(pos, h, cont, &list);
        self.end_generated = self.moves.len();
        self.end_cur = self.end_generated;
        // 3つの範囲に分ける（ADR-0243）。limit以上の手（すぐ並べる）、残りの良い静かな手、悪い静かな手。
        // limitが良い静かな手の下限以下なら、2つ目の範囲は空になる。
        // 範囲は点数で分かれているので、範囲ごとに並べれば全体が鍵の順に並ぶ
        let limit = QUIET_SORT_COEF * self.depth;
        let quiets = &mut self.moves[self.cur..self.end_generated];
        let n1 = partition(quiets, |e| e.v() >= limit);
        let n2 = if limit > GOOD_QUIET_THRESHOLD + 1 {
            n1 + partition(&mut quiets[n1..], |e| e.v() > GOOD_QUIET_THRESHOLD)
        } else {
            n1
        };
        insertion_sort_key(&mut quiets[..n1]);
        self.sorted_to = self.cur + n1;
        self.end_good_quiets = self.cur + n2;
    }

    /// 王手回避を生成して並べる（movepick.cpp:661-668）。
    /// 切り出す理由は `init_captures` と同じ。
    #[inline(never)]
    fn init_evasions(&mut self, pos: &SearchBoard, h: &Histories, cont: &[usize; 6]) {
        let mut list = MoveList::default();
        pos.generate(GenType::Evasions, false, &mut list);
        self.moves.clear();
        self.score_evasions(pos, h, cont, &list);
        self.cur = 0;
        self.end_cur = self.moves.len();
        insertion_sort_key(&mut self.moves);
        self.sorted_to = usize::MAX;
    }

    /// 直前に `next` が返した手が置換表の手か（生成した列の手でないか）。
    ///
    /// 置換表の手を返した直後だけ、段は生成の準備（`*Init`）に止まっている。生成した手を返すときは
    /// 準備の段を通り過ぎているので、段を見るだけで決まる。配列盤の生成は合法手だけを出すので、
    /// 探索はこれを見て合法性の判定を省く（ADR-0241の第15回）。
    #[inline(always)]
    pub fn last_from_tt(&self) -> bool {
        matches!(
            self.stage,
            Stage::CaptureInit | Stage::EvasionInit | Stage::QCaptureInit | Stage::ProbCutInit
        )
    }

    /// 呼ばれるたびに擬似合法手を1つ返す（movepick.cpp:456-695）。
    /// contは1手前から6手前までのcontinuation historyの面。
    pub fn next(&mut self, pos: &SearchBoard, h: &Histories, cont: &[usize; 6]) -> Option<Move> {
        loop {
            match self.stage {
                Stage::MainTt => {
                    self.stage = Stage::CaptureInit;
                    return Some(self.tt_move);
                }
                Stage::EvasionTt => {
                    self.stage = Stage::EvasionInit;
                    return Some(self.tt_move);
                }
                Stage::QsearchTt => {
                    self.stage = Stage::QCaptureInit;
                    return Some(self.tt_move);
                }
                Stage::ProbCutTt => {
                    self.stage = Stage::ProbCutInit;
                    return Some(self.tt_move);
                }
                Stage::CaptureInit | Stage::QCaptureInit | Stage::ProbCutInit => {
                    let init_stage = self.stage;
                    self.init_captures(pos, h);
                    self.stage = match init_stage {
                        Stage::CaptureInit => Stage::GoodCapture,
                        Stage::QCaptureInit => Stage::QCapture,
                        _ => Stage::ProbCut,
                    };
                }
                Stage::GoodCapture => {
                    // 損な取る手はendBadCapturesへ寄せて後回しにする。
                    // 閾値はスコアに応じて動く（movepick.cpp:512）
                    while self.cur < self.end_cur {
                        let e = self.moves[self.cur];
                        if e.m() != self.tt_move {
                            if pos.see_ge(e.m(), -e.v() / 18) {
                                self.cur += 1;
                                return Some(e.m());
                            }
                            self.moves.swap(self.end_bad_captures, self.cur);
                            self.end_bad_captures += 1;
                        }
                        self.cur += 1;
                    }
                    self.stage = Stage::QuietInit;
                }
                Stage::QuietInit => {
                    if !self.skip_quiets {
                        self.init_quiets(pos, h, cont);
                    }
                    self.stage = Stage::GoodQuiet;
                }
                Stage::GoodQuiet => {
                    if !self.skip_quiets
                        && let Some(m) = self.select_good_quiet()
                    {
                        return Some(m);
                    }
                    // 悪い取る手を返す準備。バッファ先頭の領域を読み直す
                    self.cur = 0;
                    self.end_cur = self.end_bad_captures;
                    self.stage = Stage::BadCapture;
                }
                Stage::BadCapture => {
                    if let Some(m) = self.select(|_| true) {
                        return Some(m);
                    }
                    // 悪い静かな手を返す準備。良い静かな手の段を終えているので、
                    // end_good_quiets から先は悪い静かな手だけである（ADR-0243）
                    self.cur = self.end_good_quiets;
                    self.end_cur = self.end_generated;
                    self.stage = Stage::BadQuiet;
                }
                Stage::BadQuiet => {
                    if self.skip_quiets {
                        self.stage = Stage::Done;
                        return None;
                    }
                    let m = self.select(|_| true);
                    if m.is_none() {
                        self.stage = Stage::Done;
                    }
                    return m;
                }
                Stage::EvasionInit => {
                    self.init_evasions(pos, h, cont);
                    self.stage = Stage::Evasion;
                }
                Stage::Evasion => {
                    let m = self.select(|_| true);
                    if m.is_none() {
                        self.stage = Stage::Done;
                    }
                    return m;
                }
                Stage::QCapture => {
                    // 参照実装は取る手を条件なしに良い順で返す（movepick.cpp:679-682）。
                    // 損な取り合いの切り捨ては静止探索側のSEE下限が担う
                    let m = self.select(|_| true);
                    if m.is_none() {
                        self.stage = Stage::Done;
                    }
                    return m;
                }
                Stage::ProbCut => {
                    // 閾値以上のSEEを持つ取る手だけを良い順に返す
                    // （movepick.cpp:684-685）
                    let th = self.threshold;
                    let m = self.select(|e| pos.see_ge(e.m(), th));
                    if m.is_none() {
                        self.stage = Stage::Done;
                    }
                    return m;
                }
                Stage::Done => return None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 共有historyの表がスレッド数に比例して伸びること
    /// （ADR-0162。history.h:151-153）。
    #[test]
    fn shared_history_scales_with_threads() {
        let one = SharedHistories::new(1);
        let four = SharedHistories::new(4);
        assert_eq!(four.pawn.mask + 1, (one.pawn.mask + 1) * 4);
        assert_eq!(four.corr.mask + 1, (one.corr.mask + 1) * 4);
        // 2の冪でないスレッド数は切り上げる（参照実装は2の冪を要求する）
        let three = SharedHistories::new(3);
        assert_eq!(three.pawn.mask + 1, (one.pawn.mask + 1) * 4);
    }

    /// 共有historyへの更新が、別の持ち主から読めること（ADR-0162）。
    /// スレッドローカルのままだと、この読み出しが初期値のままになる。
    #[test]
    fn shared_history_is_visible_across_owners() {
        let shared = Arc::new(SharedHistories::new(2));
        let writer = Histories::new(Arc::clone(&shared));
        let reader = Histories::new(Arc::clone(&shared));

        let pc = Piece::new(Color::Black, PieceType::PAWN);
        let to = Square::from_index(40);
        let slot = writer.shared.pawn.slot(0x1234_5678);
        let before = reader.shared.pawn.get(slot, pc, to);
        writer.shared.pawn.update(slot, pc, to, D_PAWN);
        assert!(
            reader.shared.pawn.get(slot, pc, to) > before,
            "共有していれば書き手の更新が読み手にも見える"
        );
    }

    fn mk(v: i32, bits: u32) -> ExtMove {
        ExtMove::new(Move::from_bits(bits), v)
    }

    /// 手と点数を鍵に詰めて、そのまま取り出せること（ADR-0243）。
    #[test]
    fn ext_move_roundtrip() {
        for v in [i32::MIN, -14000, -1, 0, 1, 1 << 28, i32::MAX] {
            for bits in [0u32, 1, 0x1234_5678, u32::MAX] {
                let e = mk(v, bits);
                assert_eq!(e.v(), v);
                assert_eq!(e.m().bits(), bits);
            }
        }
    }

    /// 条件を満たす手だけが先頭に集まり、手の集合が保たれること。
    #[test]
    fn partition_moves_matching_to_front() {
        let src = [(5, 3), (-100, 1), (30, 2), (5, 1), (-50, 4), (12, 9)];
        let mut a: Vec<ExtMove> = src.iter().map(|&(v, b)| mk(v, b)).collect();
        let n = partition(&mut a, |e| e.v() >= 0);
        assert_eq!(n, 4);
        assert!(a[..n].iter().all(|e| e.v() >= 0));
        assert!(a[n..].iter().all(|e| e.v() < 0));
        let mut got: Vec<u64> = a.iter().map(|e| e.k).collect();
        let mut exp: Vec<u64> = src.iter().map(|&(v, b)| mk(v, b).k).collect();
        got.sort_unstable();
        exp.sort_unstable();
        assert_eq!(got, exp);
    }

    /// `select_top` を先頭から繰り返すと、全部を鍵の順に並べた形と一致すること。
    #[test]
    fn select_top_repeated_equals_full_sort() {
        let mut s: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut rnd = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        for _ in 0..2000 {
            let n = (rnd() % 100) as usize;
            let mut a: Vec<ExtMove> = (0..n)
                .map(|i| {
                    mk(
                        (rnd() % 64) as i32 - 32,
                        (rnd() as u32 & 0xffff_0000) | i as u32,
                    )
                })
                .collect();
            let mut exp: Vec<u64> = a.iter().map(|e| e.k).collect();
            exp.sort_unstable_by(|x, y| y.cmp(x));
            let mut cur = 0;
            while cur < n {
                cur += select_top(&mut a[cur..]);
            }
            assert_eq!(a.iter().map(|e| e.k).collect::<Vec<_>>(), exp);
        }
    }

    /// 入力の並びが違っても、並べた結果が同じになること（生成の順に依存しない）。
    #[test]
    fn insertion_sort_key_ignores_input_order() {
        let src = [(3, 7), (1, 2), (3, 1), (9, 5), (1, 1), (-2, 8)];
        let mut a: Vec<ExtMove> = src.iter().map(|&(v, b)| mk(v, b)).collect();
        let mut b: Vec<ExtMove> = src.iter().rev().map(|&(v, b)| mk(v, b)).collect();
        insertion_sort_key(&mut a);
        insertion_sort_key(&mut b);
        let key = |x: &Vec<ExtMove>| x.iter().map(|e| e.k).collect::<Vec<_>>();
        assert_eq!(key(&a), key(&b));
        assert_eq!(
            a.iter().map(|e| (e.v(), e.m().bits())).collect::<Vec<_>>(),
            vec![(9, 5), (3, 1), (3, 7), (1, 1), (1, 2), (-2, 8)]
        );
    }

    /// gravity方式の更新が値域を守ること（history.h:91）。
    #[test]
    fn stats_update_stays_in_range() {
        for d in [D_BUTTERFLY, D_CAPTURE, D_PIECE_TO, D_CORRECTION] {
            let mut e: i16 = 0;
            for _ in 0..200 {
                stats_update(&mut e, d * 2, d);
                assert!(i32::from(e).abs() <= d);
            }
            assert_eq!(i32::from(e), d);
            for _ in 0..400 {
                stats_update(&mut e, -d * 2, d);
                assert!(i32::from(e).abs() <= d);
            }
            assert_eq!(i32::from(e), -d);
        }
    }
}
