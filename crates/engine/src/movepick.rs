//! 指し手オーダリングとhistoryのテーブル群（ADR-0025, 0109）。
//!
//! MovePickerは段階生成の状態機械。参照実装（やねうら王）の
//! `movepick.cpp` の遷移をそのまま移してある。カットが早いノードでは
//! Quietsの生成自体を省く。borrow衝突を避けるため、next()は毎回
//! &Positionと&Historiesを受け取る。
//!
//! テーブルの次元・値域・初期値は参照実装の `history.h` と
//! `yaneuraou-search.cpp` を出典とする（ADR-0074）。

use himawari_core::{
    Bitboard, Color, GenType, Move, MoveList, Piece, PieceType, Position, Square, generate,
    piece_value,
};

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

/// pawn history（[歩構造キー 8192][移動後の駒 32][移動先 81]。history.h:265）。
/// 参照実装はスレッド共有のatomicだが、ここではスレッドローカルに持つ。
pub struct PawnHistory {
    table: Box<[i16]>,
}

impl Default for PawnHistory {
    fn default() -> Self {
        PawnHistory {
            table: vec![INIT_PAWN; PAWN_HISTORY_BASE_SIZE * PIECE_NB * SQUARE_NB]
                .into_boxed_slice(),
        }
    }
}

impl PawnHistory {
    /// 歩構造キーからスロットを引く（history.h:370-372）。
    #[inline]
    pub fn slot(pawn_key: u64) -> usize {
        (pawn_key as usize & (PAWN_HISTORY_BASE_SIZE - 1)) * PIECE_NB * SQUARE_NB
    }

    #[inline]
    pub fn get(&self, slot: usize, pc: Piece, to: Square) -> i32 {
        i32::from(self.table[slot + pc.index() * SQUARE_NB + to.index()])
    }

    pub fn update(&mut self, slot: usize, pc: Piece, to: Square, bonus: i32) {
        stats_update(
            &mut self.table[slot + pc.index() * SQUARE_NB + to.index()],
            bonus,
            D_PAWN,
        );
    }

    pub fn clear(&mut self) {
        self.table.fill(INIT_PAWN);
    }
}

/// 静的評価の系統誤差を補正する履歴（ADR-0046, 0109）。
///
/// 参照実装のUnifiedCorrectionHistory（history.h:337-339）に対応する。
/// 1本の表を4系統（歩・小駒・先手非歩・後手非歩）で共有し、系統ごとに
/// 別のキーで引く。添字は `[キー下位16bit][手番][系統]`。
pub struct CorrectionHistory {
    table: Box<[i16]>,
}

/// correction historyの系統（history.h:296-309のCorrectionBundle）。
const CORR_PAWN: usize = 0;
const CORR_MINOR: usize = 1;
const CORR_NON_PAWN_BLACK: usize = 2;
const CORR_NON_PAWN_WHITE: usize = 3;
/// 1スロットが持つ系統数。
const CORR_KINDS: usize = 4;

impl Default for CorrectionHistory {
    fn default() -> Self {
        CorrectionHistory {
            table: vec![0i16; CORRHIST_BASE_SIZE * 2 * CORR_KINDS].into_boxed_slice(),
        }
    }
}

impl CorrectionHistory {
    #[inline]
    fn index(key: u64, stm: usize, kind: usize) -> usize {
        ((key as usize & (CORRHIST_BASE_SIZE - 1)) * 2 + stm) * CORR_KINDS + kind
    }

    #[inline]
    fn get(&self, key: u64, stm: usize, kind: usize) -> i32 {
        i32::from(self.table[Self::index(key, stm, kind)])
    }

    fn update(&mut self, key: u64, stm: usize, kind: usize, bonus: i32) {
        stats_update(
            &mut self.table[Self::index(key, stm, kind)],
            bonus,
            D_CORRECTION,
        );
    }

    /// 4系統の合成前の生の値を取り出す（yaneuraou-search.cpp:728-731）。
    /// 返り値は (歩, 小駒, 先手非歩, 後手非歩)。
    #[inline]
    pub fn probe(&self, pos: &Position) -> (i32, i32, i32, i32) {
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
    pub fn update_all(&mut self, pos: &Position, bonus: i32) {
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

    pub fn clear(&mut self) {
        self.table.fill(0);
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

    #[inline]
    pub fn get_move(&self, base: usize, m: Move) -> i32 {
        self.get(base, m.piece_after(), m.to())
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
    pub pawn: PawnHistory,
    pub cont: ContinuationHistory,
    pub corr: CorrectionHistory,
    pub corr_cont: ContinuationCorrectionHistory,
    pub tt_move: TtMoveHistory,
}

impl Histories {
    /// 対局間のリセット（yaneuraou-search.cpp:2139-2176）。
    pub fn clear(&mut self) {
        self.main.clear();
        self.capture.clear();
        self.pawn.clear();
        self.cont.clear();
        self.corr.clear();
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
#[derive(Clone, Copy)]
struct ExtMove {
    m: Move,
    v: i32,
}

/// 良い静かな手とみなすスコアの下限（movepick.cpp:459）。
const GOOD_QUIET_THRESHOLD: i32 = -14000;
/// 静かな手の部分ソートの閾値係数（movepick.cpp:605）。
const QUIET_SORT_COEF: i32 = -3560;
/// 静かな王手へのボーナスと、その資格を測るSEEの下限（movepick.cpp:371）。
const CHECK_BONUS: i32 = 16384;
const CHECK_SEE_MARGIN: i32 = -75;

/// limit以上のスコアを持つ指し手だけを降順に並べる（movepick.cpp:91-105）。
/// limit未満の並びは不定になる。
fn partial_insertion_sort(a: &mut [ExtMove], limit: i32) {
    let mut sorted_end = 0usize;
    for p in 1..a.len() {
        if a[p].v >= limit {
            let tmp = a[p];
            sorted_end += 1;
            a[p] = a[sorted_end];
            let mut q = sorted_end;
            while q != 0 && a[q - 1].v < tmp.v {
                a[q] = a[q - 1];
                q -= 1;
            }
            a[q] = tmp;
        }
    }
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
}

impl MovePicker {
    fn make(stage: Stage, tt_move: Move, depth: i32, ply: usize, threshold: i32) -> Self {
        MovePicker {
            stage,
            tt_move,
            depth,
            ply,
            skip_quiets: false,
            moves: Vec::with_capacity(64),
            cur: 0,
            end_cur: 0,
            end_bad_captures: 0,
            end_captures: 0,
            end_generated: 0,
            threshold,
        }
    }

    /// 通常探索・静止探索用（movepick.cpp:120-202）。depth <= 0で静止探索。
    pub fn new(pos: &Position, tt_move: Move, depth: i32, ply: usize) -> Self {
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
        Self::make(stage, tt_move, depth, ply, 0)
    }

    /// ProbCut用（movepick.cpp:204-252）。SEEが閾値以上の取る手だけを返す。
    /// 置換表の手は取る手でありさえすれば、SEEを見ずに先に返す
    /// （movepick.cpp:245-248）。ProbCutは王手中に呼ばれない
    pub fn new_probcut(pos: &Position, tt_move: Move, threshold: i32) -> Self {
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
        Self::make(stage, tt_move, 0, 0, threshold)
    }

    /// 静かな手をもう返さないよう伝える（movepick.cpp:697）。
    pub fn skip_quiet_moves(&mut self) {
        self.skip_quiets = true;
    }

    /// 条件を満たす次の手を返す（movepick.cpp:436-445）。TT手は返さない。
    #[inline]
    fn select(&mut self, pred: impl Fn(&ExtMove) -> bool) -> Option<Move> {
        while self.cur < self.end_cur {
            let e = self.moves[self.cur];
            self.cur += 1;
            if e.m != self.tt_move && pred(&e) {
                return Some(e.m);
            }
        }
        None
    }

    /// 取る手のスコア（movepick.cpp:341-342）。
    /// 取った駒の価値を7倍し、capture historyを足す。倍率7は、あとで
    /// `see_ge(m, -value/18)` に渡すためにSEEのスケールへ合わせたもの。
    fn score_captures(&mut self, pos: &Position, h: &Histories, list: &MoveList) {
        for &m in list {
            let pc = m.piece_after();
            let to = m.to();
            let captured = pos.piece_on(to).piece_type();
            let v = h.capture.get(pc, to, captured) + 7 * piece_value(captured);
            self.moves.push(ExtMove { m, v });
        }
    }

    /// 静かな手のスコア（movepick.cpp:362-393）。
    fn score_quiets(&mut self, pos: &Position, h: &Histories, cont: &[usize; 6], list: &MoveList) {
        let pawn_slot = PawnHistory::slot(pos.pawn_key());
        let us = pos.side_to_move();
        // 直接王手になるマスは駒種ごとに1回だけ引く
        let mut check_sq: [Option<Bitboard>; PIECE_TYPE_NB] = [None; PIECE_TYPE_NB];
        for &m in list {
            let pc = m.piece_after();
            let to = m.to();
            let mut v = 2 * h.main.get(us, m);
            v += 2 * h.pawn.get(pawn_slot, pc, to);
            v += h.cont.get(cont[0], pc, to);
            v += h.cont.get(cont[1], pc, to);
            v += h.cont.get(cont[2], pc, to);
            v += h.cont.get(cont[3], pc, to);
            v += h.cont.get(cont[5], pc, to);
            // 王手になる手へのボーナス
            let pt = pc.piece_type();
            let cs = *check_sq[pt.index()].get_or_insert_with(|| pos.check_squares(pt));
            if cs.test(to) && pos.see_ge(m, CHECK_SEE_MARGIN) {
                v += CHECK_BONUS;
            }
            if self.ply < LOW_PLY_HISTORY_SIZE {
                v += 8 * h.low_ply.get(self.ply, m) / (1 + self.ply as i32);
            }
            self.moves.push(ExtMove { m, v });
        }
    }

    /// 王手回避のスコア（movepick.cpp:396-421）。
    fn score_evasions(
        &mut self,
        pos: &Position,
        h: &Histories,
        cont: &[usize; 6],
        list: &MoveList,
    ) {
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
            self.moves.push(ExtMove { m, v });
        }
    }

    /// 呼ばれるたびに擬似合法手を1つ返す（movepick.cpp:456-695）。
    /// contは1手前から6手前までのcontinuation historyの面。
    pub fn next(&mut self, pos: &Position, h: &Histories, cont: &[usize; 6]) -> Option<Move> {
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
                    let mut list = MoveList::default();
                    generate(pos, GenType::Captures, false, &mut list);
                    self.moves.clear();
                    self.score_captures(pos, h, &list);
                    self.cur = 0;
                    self.end_bad_captures = 0;
                    self.end_captures = self.moves.len();
                    self.end_generated = self.end_captures;
                    self.end_cur = self.end_captures;
                    // 取る手は数が多くないので全数ソートでよい
                    partial_insertion_sort(&mut self.moves, i32::MIN);
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
                        if e.m != self.tt_move {
                            if pos.see_ge(e.m, -e.v / 18) {
                                self.cur += 1;
                                return Some(e.m);
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
                        let mut list = MoveList::default();
                        generate(pos, GenType::Quiets, false, &mut list);
                        self.score_quiets(pos, h, cont, &list);
                        self.end_generated = self.moves.len();
                        self.end_cur = self.end_generated;
                        partial_insertion_sort(
                            &mut self.moves[self.cur..self.end_cur],
                            QUIET_SORT_COEF * self.depth,
                        );
                    }
                    self.stage = Stage::GoodQuiet;
                }
                Stage::GoodQuiet => {
                    if !self.skip_quiets
                        && let Some(m) = self.select(|e| e.v > GOOD_QUIET_THRESHOLD)
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
                    // 悪い静かな手を返す準備
                    self.cur = self.end_captures;
                    self.end_cur = self.end_generated;
                    self.stage = Stage::BadQuiet;
                }
                Stage::BadQuiet => {
                    if self.skip_quiets {
                        self.stage = Stage::Done;
                        return None;
                    }
                    let m = self.select(|e| e.v <= GOOD_QUIET_THRESHOLD);
                    if m.is_none() {
                        self.stage = Stage::Done;
                    }
                    return m;
                }
                Stage::EvasionInit => {
                    let mut list = MoveList::default();
                    generate(pos, GenType::Evasions, false, &mut list);
                    self.moves.clear();
                    self.score_evasions(pos, h, cont, &list);
                    self.cur = 0;
                    self.end_cur = self.moves.len();
                    partial_insertion_sort(&mut self.moves, i32::MIN);
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
                    let m = self.select(|e| pos.see_ge(e.m, th));
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

    /// 部分挿入ソートがlimit以上の要素を降順に並べること（movepick.cpp:91）。
    #[test]
    fn partial_insertion_sort_orders_above_limit() {
        let mk = |v: i32| ExtMove { m: Move::NONE, v };
        let mut a: Vec<ExtMove> = [5, -100, 30, 7, -50, 12].iter().map(|&v| mk(v)).collect();
        partial_insertion_sort(&mut a, 0);
        // limit以上の4要素が先頭に降順で並ぶ
        assert_eq!(
            a.iter().take(4).map(|e| e.v).collect::<Vec<_>>(),
            vec![30, 12, 7, 5]
        );
        // 残りはlimit未満の2要素（順序は不定）
        let mut rest: Vec<i32> = a.iter().skip(4).map(|e| e.v).collect();
        rest.sort_unstable();
        assert_eq!(rest, vec![-100, -50]);
    }

    /// limitをi32::MINにすると全数が降順に並ぶこと。
    #[test]
    fn partial_insertion_sort_full() {
        let mk = |v: i32| ExtMove { m: Move::NONE, v };
        let mut a: Vec<ExtMove> = [3, 1, 4, 1, 5, 9, 2, 6].iter().map(|&v| mk(v)).collect();
        partial_insertion_sort(&mut a, i32::MIN);
        assert_eq!(
            a.iter().map(|e| e.v).collect::<Vec<_>>(),
            vec![9, 6, 5, 4, 3, 2, 1, 1]
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
