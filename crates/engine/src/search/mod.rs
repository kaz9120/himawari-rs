//! 探索v1。
//!
//! fail-softのalpha-beta＋反復深化＋aspiration window。
//! PVは三角配列（Vecの連結）。詰みスコアはTT境界でply補正する。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};

use himawari_core::{Color, Move, MoveList, Piece, PieceType, Position, Repetition, Square};

use crate::board::SearchBoard;
use crate::eval::Evaluator;
use crate::movepick::{
    ContinuationCorrectionHistory, ContinuationHistory, Histories, LOW_PLY_HISTORY_SIZE, MoveBuf,
    MovePicker, SharedHistories,
};
use crate::timeman::{IterationStats, Limits, TimeManager};
use crate::tt::{Bound, EvalHash, Tt};
#[allow(clippy::wildcard_imports)]
use crate::tunables::*;
use crate::value::{
    MAX_PLY, PAWN_VALUE, VALUE_DRAW, VALUE_INFINITE, VALUE_MATE_IN_MAX_PLY, VALUE_MATED_IN_MAX_PLY,
    VALUE_NONE, VALUE_SUPERIOR, VALUE_ZERO, Value, mate_in, mated_in, value_from_tt, value_to_tt,
};

mod node;
mod params;
mod prune;
mod qsearch;
mod root;
mod stats;

#[allow(clippy::wildcard_imports)]
use params::*;

/// スレッド間の共有状態。
pub struct Shared {
    pub stop: AtomicBool,
    /// 探索を破棄した（反復の途中で止めた）ことを示す（thread.h:296-301）。
    /// 中断した反復のスコアは信用できないので、前の反復へ戻す判断に使う
    pub aborted_search: AtomicBool,
    /// go ponder中か（yaneuraou-search.h:203-214の `SearchManager::ponder`）。
    /// 真の間は時間で探索を止めない。"ponderhit" で偽になる。"stop" では
    /// 変えない（参照実装も同じ。stopは `stop` フラグ側で止まる）
    pub ponder: AtomicBool,
    /// "ponderhit" した時刻の、go受領時刻からの経過[ms]（timeman.h:120の
    /// `ponderhitTime - startTime`）。"ponderhit" を受けるまでは0
    pub ponderhit_offset: AtomicI64,
    pub nodes: AtomicU64,
    /// 反復深化で深さが増えているか（G9。yaneuraou-search.h:232）。
    /// メインが時間の余りから決め、全スレッドが次の反復の頭で読む。
    /// 偽なら同じ深さを掘り直したとみなし `search_again_counter` が増える
    pub increase_depth: AtomicBool,
    /// rootの最善手が変わった回数（G9。yaneuraou-search.h:539）。
    /// 参照実装はスレッドごとの原子変数を持ち、メインが毎反復で合算して
    /// 0へ戻す。本エンジンは合算先を1つにまとめ、`swap(0)` で汲み出す
    pub best_move_changes: AtomicU64,
    pub tt: Tt,
    /// 評価値キャッシュ。全スレッド共有、new_gameでクリア。
    pub eval_hash: EvalHash,
    /// 全スレッドで共有するhistory。pawn historyと
    /// correction historyの2面を持ち、表はスレッド数に比例して伸びる
    pub hists: Arc<SharedHistories>,
}

impl Shared {
    pub fn new(hash_mb: usize) -> Shared {
        Shared::with_threads(hash_mb, 1)
    }

    pub fn with_threads(hash_mb: usize, threads: usize) -> Shared {
        Shared {
            hists: Arc::new(SharedHistories::new(threads)),
            stop: AtomicBool::new(false),
            aborted_search: AtomicBool::new(false),
            ponder: AtomicBool::new(false),
            ponderhit_offset: AtomicI64::new(0),
            nodes: AtomicU64::new(0),
            increase_depth: AtomicBool::new(true),
            best_move_changes: AtomicU64::new(0),
            tt: Tt::new(hash_mb),
            eval_hash: EvalHash::new(),
        }
    }
}

/// 探索中の報告。反復深化1周分の結果と、rootで今読んで
/// いる手の2種類がある。呼び出し側はどちらもUSIのinfo行へ落とす。
pub enum SearchInfo {
    /// 反復深化1周分（MultiPVでは1ラインごとに1回）。
    Iteration(IterInfo),
    /// aspiration窓を外れた途中経過。確定値ではないため、
    /// 消費側はUSIの `lowerbound` / `upperbound` を付けて出す。
    Bound(IterInfo, ScoreBound),
    /// rootで今読んでいる手。長考中の可視化に使う。
    CurrMove { depth: u32, mv: Move },
    /// 診断の1行（`info string`）。時間管理の計画と実績など、指し手の
    /// 判断には使わないが、事後の調査で要る情報を出す。
    Note(String),
}

/// aspiration窓を外れたときのスコアの確からしさ。
/// USIの `lowerbound` / `upperbound` に対応する。
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum ScoreBound {
    /// fail high。実際の評価はこの値以上。
    Lower,
    /// fail low。実際の評価はこの値以下。
    Upper,
}

/// 反復深化1周分の報告（MultiPVでは1ラインごとに1回）。
pub struct IterInfo {
    pub depth: u32,
    /// 静止探索を含めて到達した最大ply。
    pub seldepth: u32,
    /// 1始まりのライン番号。MultiPV=1のときは0（info出力で省略）。
    pub multipv: usize,
    pub score: Value,
    pub pv: Vec<Move>,
    pub nodes: u64,
    pub elapsed_ms: u64,
    pub hashfull: usize,
}

/// root手1つの探索状態。
pub struct RootMove {
    pub mv: Move,
    pub score: Value,
    pub prev_score: Value,
    pub pv: Vec<Move>,
    /// この手の探索に費やしたノード数（G9。search.h:129）。
    /// メインワーカーのローカル値のみ。**go全体の累計で、イテレーション
    /// ごとには戻さない。** highBestMoveEffortが読む比の分母も同じく
    /// go全体の累計ノード数である（S:1969-1970）
    pub effort: u64,
    /// スコアの移動平均（G9。search.h:139、yaneuraou-search.cpp:4105-4106）。
    /// aspirationの窓の中心に使う。前深さの生スコアより揺れが小さい。
    /// 初期値の `-VALUE_INFINITE` は「まだ値がない」ことを表す番兵
    pub average_score: Value,
    /// スコアの二乗平均（G9。search.h:142、yaneuraou-search.cpp:4108-4110）。
    /// 符号を保つため `value * |value|` を平均する。aspirationの初期窓幅が
    /// この絶対値に比例して広がる。初期値の `-VALUE_INFINITE^2` は番兵で、
    /// **この値のまま窓幅を計算すると窓が全開になる。** 深さ1で
    /// aspirationを事実上無効にする仕掛けである
    pub mean_squared_score: Value,
}

/// `RootMove::mean_squared_score` の番兵（search.h:142）。
const MEAN_SQUARED_NONE: Value = -(VALUE_INFINITE * VALUE_INFINITE);

/// goをまたいでメインスレッドが持ち越す記憶（G9。yaneuraou-search.h:207-247）。
/// 参照実装はSearchManagerのメンバーで、`isready` でクリアされる。
/// 本エンジンはスレッドループが持ち、`usinewgame` でクリアする
#[derive(Clone, Copy)]
pub struct MainMemory {
    /// 前回のgoの最終スコア（yaneuraou-search.h:215）。`iter_value` の
    /// 初期値に使う。`VALUE_INFINITE` は「前回がない」ことを表す番兵
    pub best_previous_score: Value,
    /// 前回のgoの最善手の `average_score`（yaneuraou-search.h:216）。
    /// 思考時間のfallingEvalが読む。番兵は同じく `VALUE_INFINITE` で、
    /// **番兵のままだとfallingEvalが上限に張り付き、初手に時間を多く使う。**
    /// 参照実装のコメントが明示する意図的な挙動である（S:281-285）
    pub best_previous_average_score: Value,
    /// 前回のgoのtimeReduction（yaneuraou-search.h:211）
    pub previous_time_reduction: f64,
    /// 前回のgoのgame_ply（yaneuraou-search.h:247）。手番が入れ替わって
    /// いないかの検出に使う
    pub last_game_ply: u16,
}

impl Default for MainMemory {
    /// 参照実装の `YaneuraOuEngine::clear()`（S:281-292）と同じ初期値。
    fn default() -> Self {
        MainMemory {
            best_previous_score: VALUE_INFINITE,
            best_previous_average_score: VALUE_INFINITE,
            previous_time_reduction: 0.85,
            last_game_ply: 0,
        }
    }
}

#[derive(Clone)]
pub struct SearchResult {
    pub best: Move,
    pub score: Value,
    /// 相手の予測応手（PVの2手目。なければNONE）。
    pub ponder: Move,
    /// `root_moves[0]` の生スコア（G10。S:610-611の投票が読む）。
    /// USIへ出す `score` と違い、頭打ちや窓外れの整形をしていない
    pub root_score: Value,
    /// `root_moves[0].average_score`（G10。S:1250）。
    /// best threadのものを次のgoへ持ち越す
    pub root_average_score: Value,
    /// `root_moves[0].pv`（G10。S:625-626の投票が読む）。
    pub pv: Vec<Move>,
    /// 確定した最後のイテレーションの深さ（G10。S:618の投票が読む）。
    pub completed_depth: u32,
}

/// plyごとの探索状態。
///
/// 参照実装のStackに対応する。statScore・moveCount・ttPvなどは、
/// 読む側を入れる群で足す。
#[derive(Clone, Copy)]
struct StackEntry {
    /// このplyで指した手。null moveはMove::NULL、rootの手前はMove::NONE
    current_move: Move,
    /// singular検証探索中の除外手
    excluded_move: Move,
    /// 静的評価。王手中はVALUE_NONE
    static_eval: Value,
    /// このplyで手番側に王手がかかっていたか（G1）。continuation historyの
    /// 面の選択と、更新する段数の打ち切りに使う
    in_check: bool,
    /// このplyで指した手が条件手になるcontinuation historyの面（G1）。
    /// 参照実装の `Stack::continuationHistory` に対応する
    cont_base: usize,
    /// 同じくcontinuation correction historyの面（G1）。
    cont_corr_base: usize,
    /// このplyで今何手目を調べているか（G1。yaneuraou-search.cpp:3520）。
    /// 1手前の値をhistoryの更新条件が読む
    move_count: u32,
    /// このplyで置換表にヒットしたか（G1。yaneuraou-search.cpp:2623）。
    /// 同じく1手前の値を読む
    tt_hit: bool,
    /// 置換表にPVノードとして記録された値か（G2。yaneuraou-search.cpp:2657）。
    /// LMRのリダクション2項が読み、置換表へ書き戻す
    tt_pv: bool,
    /// このplyでβカットした回数（G2。yaneuraou-search.cpp:4214）。
    /// 親が次plyの値を読む。多いほどリダクションを増やす
    cutoff_cnt: i32,
    /// このplyのLMRが削った量（G2。yaneuraou-search.cpp:3980）。
    /// 子が `priorReduction` として読み、深さの事後補正に使う（G4）
    reduction: i32,
    /// このplyで今調べている手の履歴の強さ（G2。yaneuraou-search.cpp:3924-3932）。
    /// リダクションの減算と、子でのhistory更新量の2方向へ効く
    stat_score: i32,
    /// 前回の反復深化のPV上にいるか（G3。yaneuraou-search.cpp:2370-2372）。
    /// PVノードでここが真の間は静かな手の浅い枝刈りを抑え、前回のPVを壊さない
    follow_pv: bool,
}

/// Stackの前方余白。ss-6まで境界検査なしで参照するために置く。
const STACK_OFFSET: usize = 7;

/// Stackの要素数。前方の余白と、`ply + 2` まで書く後方の余裕を含む。
/// 固定長にすることで、`stack[ply + STACK_OFFSET]` の境界検査が
/// コンパイル時定数との比較になる
const STACK_LEN: usize = MAX_PLY + 10;

/// Stackの初期値。全plyをこれで埋める。
const STACK_INIT: StackEntry = StackEntry {
    current_move: Move::NONE,
    excluded_move: Move::NONE,
    static_eval: VALUE_NONE,
    in_check: false,
    // 指し手のないplyは番兵の面を指す
    cont_base: ContinuationHistory::SENTINEL,
    cont_corr_base: 0,
    move_count: 0,
    tt_hit: false,
    tt_pv: false,
    cutoff_cnt: 0,
    reduction: 0,
    stat_score: 0,
    follow_pv: false,
};

/// 置換表から読んだ値。`probe_tt` が組み立て、
/// TTカット・枝刈り・延長・リダクションが繰り返し読む。
#[derive(Clone, Copy)]
struct TtInfo {
    /// probeの生データ。eval欄の再利用とヒットの有無の判定が読む
    hit: Option<crate::tt::TtData>,
    /// この局面で指せる形に直したTT手。指せないときは `Move::NONE`
    mv: Move,
    value: Value,
    /// ゲタを外した深さ。ヒットしないときは参照実装のDEPTH_NONE
    /// （tt.cpp:445）に合わせて -3 とする
    depth: i32,
    bound: Bound,
}

/// ムーブループの手前で決まるノードの前提。
///
/// 切り出した枝刈り・延長・リダクション・終端処理が共通して読む値を束ねる。
/// `search` の引数と局所変数をそのまま写したもので、値は組み立てたあと
/// 変わらない。ムーブループで動く `depth`・`alpha`・`improving` はここへ
/// 入れず、関数の引数で渡す。
///
/// ノードがPVかどうかはここへ入れない。`search` と同じくconst genericの
/// `PV` で受け取る。
#[derive(Clone, Copy)]
struct NodeInfo {
    ply: usize,
    cut_node: bool,
    /// PVでもcutでもないノード（yaneuraou-search.cpp:2251）。
    /// 全手を調べる見込みなのでリダクションを強める。IIRの条件も読む
    all_node: bool,
    in_check: bool,
    /// 前回の反復深化のPVを辿っているか（yaneuraou-search.cpp:2370-2372）
    follow_pv: bool,
    /// 除外手（singular extension用）。検証探索中はTT手が入る
    excluded: Move,
    key: u64,
    /// correction history補正前の静的評価。TT storeのeval欄へそのまま入る
    raw_eval: Value,
    /// correction history補正後の静的評価
    static_eval: Value,
    /// 枝刈り用の評価値（yaneuraou-search.cpp:3084-3087）。置換表の値の
    /// ほうが見積りとして適切なら差し替わる。razoringとRFPだけが読む
    eval: Value,
    /// correction historyの合成値（yaneuraou-search.cpp:3010）
    corr_value: i32,
    /// 1手前のLMRが削った量（yaneuraou-search.cpp:2552-2553）
    prior_reduction: i32,
    /// 1手前が取った駒（yaneuraou-search.cpp:2355）
    prior_capture: Piece,
    /// 1手前の移動先（yaneuraou-search.cpp:2550）。特殊手ならNone
    prev_sq: Option<Square>,
    tt: TtInfo,
    /// 置換表の手が駒を取る手か（yaneuraou-search.cpp:2671）。
    /// RFPの条件とリダクションの1項が読む
    tt_capture: bool,
}

/// ノードごとに使い回すバッファ。
///
/// 素直に書くと毎ノードで `Vec` を確保し直すので、mallocがプロファイルの
/// 2.1%を占めていた。`Worker` がply別に持ち、`std::mem::take` で貸し出して
/// 使い終わったら戻す。takeは空との交換なので確保が起きず、戻したときに
/// 容量が残る。同じplyで次に借りるときは確保も再確保も起きない。
///
/// 添字はそのバッファを所有するノードのplyである。ノードは自分のplyの
/// スロットだけを借り、子は `ply + 1` のスロットを借りるので衝突しない。
/// 同一plyへ再帰する経路（singular検証・NMP検証）は、いずれも借りる前に
/// 呼んで戻ってくるので重ならない。
///
/// 返却漏れが起きても正しさは壊れない。次に借りるときが空の `Vec` に
/// なり、確保が1回復活するだけである。
struct NodeBuffers {
    /// 最善にならなかった静かな手（`search` のquiets_searched）
    quiets: Vec<Move>,
    /// 同じく取る手（`search` のcaptures_searched）
    captures: Vec<Move>,
    /// 子ノードのPV（`search` のchild_pv）
    child_pv: Vec<Move>,
    /// `MovePicker` へ貸す採点済みの手
    moves: MoveBuf,
}

impl NodeBuffers {
    fn with_capacity() -> NodeBuffers {
        NodeBuffers {
            // 覚える手数の上限は決まっている（yaneuraou-search.cpp:4246-4256）
            quiets: Vec::with_capacity(SEARCHED_LIST_CAPACITY as usize),
            captures: Vec::with_capacity(SEARCHED_LIST_CAPACITY as usize),
            // PVが伸びるのはPVノードだけなので、伸びたぶんを残せば足りる
            child_pv: Vec::new(),
            moves: MoveBuf::with_node_capacity(),
        }
    }
}

pub struct Worker {
    /// 探索が進め戻す盤面。読み取りは `Position` のメソッドを
    /// そのまま呼べる
    pub pos: SearchBoard,
    pub evaluator: Evaluator,
    /// historyの一式。対局を通じてスレッドが持ち回る。
    pub hist: Histories,
    /// plyごとの探索状態。添字は `ply + STACK_OFFSET` で引く。
    /// 前方の余白により、ply 0でも1手前・2手前を境界検査なしで読める。
    /// 長さをコンパイル時定数にするため配列で持つ。`Vec` だと
    /// 長さがヒープ上にあり、`&mut self` を通る呼び出しのたびに読み直す
    stack: Box<[StackEntry; STACK_LEN]>,
    /// ply別の再利用バッファ。`search` と `qsearch` は
    /// `ply >= MAX_PLY` で何も借りずに返すので、長さはMAX_PLYで足りる
    node_bufs: Box<[NodeBuffers; MAX_PLY]>,
    /// このイテレーションで到達した最大ply（seldepth）。
    sel_depth: u32,
    /// このイテレーションのaspiration窓の幅（G2。yaneuraou-search.cpp:1708）。
    /// リダクションが「今の窓幅がroot窓幅の何割か」で削る量を決める
    root_delta: Value,
    /// 今の反復深化の深さ（G5。yaneuraou-search.h:557）。
    /// singularの多段化のマージンが `ply > rootDepth` で分岐に使う。
    /// rootから遠いノードでは延長を積みにくくする項である
    root_depth: u32,
    /// このplyに達するまでNMPを止める（G4。yaneuraou-search.h:543）。
    /// NMPの検証探索の中で立て、探索を抜けたら0へ戻す。再帰的な検証を
    /// 認めないための状態で、スレッドごとに持つ
    nmp_min_ply: usize,
    /// 前回の反復深化で得たPV（G3。yaneuraou-search.h:562）。
    /// 各ノードの `follow_pv` 判定が読む。goのたびに捨てる
    last_iteration_pv: Vec<Move>,
    /// 深さ1のイテレーションを終えたか。終えるまでstopを無視する。
    /// root手は生成順のままなので、深さ1の途中で打ち切ると探索して
    /// いない手を返してしまう
    depth1_done: bool,
    /// go ponder中に予算を使い切った（yaneuraou-search.h:220）。
    /// ponder中はGUIの指示があるまで止められないので、その場では止めず
    /// 予約だけしておく。ponderhit後の最初の判定で終了時刻が確定する。
    /// aspirationのfail lowで解除する（S:1783-1784）
    stop_on_ponderhit: bool,
    nodes: u64,
    shared: Arc<Shared>,
    tm: TimeManager,
    limits: Limits,
    max_moves_to_draw: u16,
    /// 検討モードのライン数。対局時は1。
    multi_pv: usize,
    root_moves: Vec<RootMove>,
    /// このワーカーの通し番号（G9。thread.h:198のthreadIdx）。
    /// aspirationの初期窓幅を `thread_idx % 8` だけずらす。参照実装が持つ
    /// 唯一の明示的なLazy SMPの多様化である
    thread_idx: usize,
    /// ワーカーの総数（G9）。bestMoveInstabilityの分母になる
    thread_count: usize,
    /// goをまたぐ記憶（G9）。スレッドループが持ち回る
    pub memory: MainMemory,
    /// rootの手番（G10）。引き分けの評価値の符号を決める
    root_color: Color,
    /// rootの手番から見た引き分けの評価値（G10。S:1002-1009）。
    /// `DrawValueBlack` / `DrawValueWhite` を歩の価値で換算した値で、
    /// `ThreadPool` が `set_draw_value` で入れる。既定は0（従来と同じ）
    draw_value_us: Value,
    /// LMRのリダクション表への参照。`Worker::new` で1回だけ受け取る。
    /// 表引きは指し手ごとに起きるので、そのたびに `OnceLock` を叩かない
    reductions: &'static [i16; REDUCTIONS_LEN],
}

impl Worker {
    // 値の持ち主がばらばらで、posとevaluatorとhistはスレッドへmoveし、sharedはArcで
    // 共有し、limitsとtmはコピーで、残り2つはoptsから読む。構造体へまとめても
    // スレッドごとに詰め直すことになる。goごとに変わる残りはset_thread・set_draw_valueで入れる
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        pos: Position,
        shared: Arc<Shared>,
        limits: Limits,
        tm: TimeManager,
        max_moves_to_draw: u16,
        multi_pv: usize,
        evaluator: Evaluator,
        hist: Histories,
    ) -> Worker {
        let root_color = pos.side_to_move();
        Worker {
            pos: SearchBoard::new(pos),
            evaluator,
            hist,
            // ヒープ上で作ってから配列へ移す。要素数はコンパイル時に
            // 決まっているので、この変換が失敗することはない
            stack: vec![STACK_INIT; STACK_LEN]
                .into_boxed_slice()
                .try_into()
                .unwrap_or_else(|_| unreachable!("要素数はSTACK_LENで固定")),
            node_bufs: std::iter::repeat_with(NodeBuffers::with_capacity)
                .take(MAX_PLY)
                .collect::<Vec<_>>()
                .into_boxed_slice()
                .try_into()
                .unwrap_or_else(|_| unreachable!("要素数はMAX_PLYで固定")),
            sel_depth: 0,
            // 0除算を避ける番兵。search_rootが毎回入れ直す
            root_delta: 1,
            root_depth: 0,
            nmp_min_ply: 0,
            last_iteration_pv: Vec::new(),
            depth1_done: false,
            stop_on_ponderhit: false,
            nodes: 0,
            shared,
            tm,
            limits,
            max_moves_to_draw,
            multi_pv: multi_pv.max(1),
            root_moves: Vec::new(),
            thread_idx: 0,
            thread_count: 1,
            memory: MainMemory::default(),
            root_color,
            draw_value_us: VALUE_ZERO,
            reductions: reductions_table(),
        }
    }

    /// 局面とaccumulatorのスタックを1手進める。
    ///
    /// `do_move` と `push` は必ず対で行う。pushを欠かすと、accumulatorが
    /// 1手前のまま計算済みとして読まれる。局面に付いて回る状態を足すときは、
    /// ここと `unmake_move` に足す。
    #[inline(always)]
    fn make_move(&mut self, m: Move) {
        // do_moveとpushの間も読み込みが進む
        self.prefetch_child(m);
        self.make_move_prefetched(m);
    }

    /// `make_move` から、置換表とeval hashの先読みを除いたもの。手を選んだ時点で
    /// `prefetch_child` を済ませた指し手ループが使う。
    #[inline(always)]
    fn make_move_prefetched(&mut self, m: Move) {
        self.pos.do_move(m);
        self.shared.hists.corr.prefetch(&self.pos);
        self.evaluator.push(&self.pos);
    }

    /// mで進めた子ノードが引く置換表とeval hashを、進める前に先読みさせる。
    #[inline(always)]
    fn prefetch_child(&self, m: Move) {
        let key = self.pos.prefetch_key(m);
        self.shared.tt.prefetch(key);
        self.shared.eval_hash.prefetch(key);
    }

    /// 進めた局面で引く置換表とeval hashとcorrection historyを先読みさせる。
    /// 子ノードがprobeに着くまでの間に読み込みを済ませる。
    #[inline(always)]
    fn prefetch_tables(&self) {
        let key = self.pos.key();
        self.shared.tt.prefetch(key);
        self.shared.eval_hash.prefetch(key);
        self.shared.hists.corr.prefetch(&self.pos);
    }

    /// `make_move` を戻す。進めたのと逆の順で戻す。
    #[inline(always)]
    fn unmake_move(&mut self, m: Move) {
        self.evaluator.pop();
        self.pos.undo_move(m);
    }

    /// null moveで1手進める。対の作法は `make_move` と同じ。
    #[inline(always)]
    fn make_null_move(&mut self) {
        self.pos.do_null_move();
        self.prefetch_tables();
        self.evaluator.push(&self.pos);
    }

    /// `make_null_move` を戻す。
    #[inline(always)]
    fn unmake_null_move(&mut self) {
        self.evaluator.pop();
        self.pos.undo_null_move();
    }

    /// 引き分けの評価値を設定する（G10。S:1002-1009）。手番別の設定値を
    /// 歩の価値で換算し、rootの手番から見た値として持つ。
    /// 相手番のleafでは符号を反転させる。非対称な探索を避けるためである
    pub fn set_draw_value(&mut self, black: i32, white: i32) {
        let v = if self.root_color == Color::Black {
            black
        } else {
            white
        };
        self.draw_value_us = v * PAWN_VALUE / 100;
    }

    /// このワーカーの通し番号と総数を渡す（G9）。aspirationの多様化と
    /// bestMoveInstabilityの分母に使う。既定は単スレッド相当の (0, 1)。
    pub fn set_thread(&mut self, idx: usize, count: usize) {
        self.thread_idx = idx;
        self.thread_count = count.max(1);
    }

    /// ヘルパーがこの反復を飛ばすか。旧Stockfish（8〜11）の
    /// skipSize/skipPhase表で、ヘルパーは番号ごとに違う周期で反復を
    /// 飛ばし、同じ深さを同時に読む重複を減らす。メインは飛ばさない。
    ///
    /// 8スレッドで同じ深さに使うノードが3.66倍から2.81倍に減り、8対8の
    /// 既定SPRTで+19.8 Elo。参照実装は2020年前後にこの表を外したが、
    /// 本エンジンでは多様化なしだと重複が大きかった
    fn skip_iteration(&self, depth: u32) -> bool {
        const SKIP_SIZE: [u32; 20] = [1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3, 4, 4, 4, 4, 4];
        const SKIP_PHASE: [u32; 20] = [0, 1, 0, 1, 2, 3, 0, 1, 2, 3, 4, 5, 6, 7, 0, 1, 2, 3, 4, 5];
        if self.thread_idx == 0 {
            return false;
        }
        let i = (self.thread_idx - 1) % 20;
        ((depth + SKIP_PHASE[i]) / SKIP_SIZE[i]) % 2 == 1
    }

    /// メインは深さ1を終えるまでstopを無視する。`iterate` は打ち切り時に
    /// `root_moves[0]` を返すが、root手は生成順に並んでいるため、深さ1の
    /// 途中で止まると探索していない手が出てしまう。
    ///
    /// ヘルパーには同じ保護を掛けない。メインはbestmoveの前に全ヘルパーの
    /// 結論を待つので、深さ1が膨れたヘルパーが1本あるとbestmoveが出ない。
    /// 2026-09-24のfloodgate戦で、time planの行の後にbestmoveが31秒出ず
    /// 切れ負けた（Issue #545）。深さ1を終えずに抜けたヘルパーの結論は、
    /// 投票から外す（`ThreadPool::conclude`）
    #[inline]
    fn stopped(&self) -> bool {
        (self.depth1_done || self.thread_idx != 0) && self.shared.stop.load(Ordering::Relaxed)
    }

    /// 定期的な時間・ノード制限の検査（S:5480-5560）。時間制限を持つのは
    /// メインワーカーだけ（ヘルパーはtmが無制限）。
    /// あわせてローカルのノード数を共有カウンタへ流し込む。
    ///
    /// 即座に止めるのは3つだけである。movetime超過、nodes超過、予約した
    /// 終了時刻の到来。`maximum()` の超過はその場では止めず、秒単位で
    /// 切り上げた終了時刻を予約する。秒ぎりぎりまで思考したほうが得だから
    #[inline]
    fn check_limits(&mut self) {
        if !self.nodes.is_multiple_of(2048) {
            return;
        }
        self.shared.nodes.fetch_add(2048, Ordering::Relaxed);
        // go ponder中はGUIの指示があるまで止めない（S:5502-5507）。
        // 時間管理そのものは働いていて、経過時間はgo受領時刻から数える
        if self.shared.ponder.load(Ordering::SeqCst) {
            return;
        }
        // 深さ1を終えるまでは止めない（S:5542の completedDepth >= 1）
        if !self.depth1_done {
            return;
        }
        let elapsed = self.tm.elapsed_ms();
        let ponderhit_offset = self.shared.ponderhit_offset.load(Ordering::SeqCst);
        if (self.limits.movetime > 0 && elapsed >= self.limits.movetime as i64)
            || (self.limits.nodes > 0 && self.nodes >= self.limits.nodes)
            // search_endは0が「未確定」を表す。負の値も予約済みとして扱う。
            // MinimumThinkingTimeを小さくするとround_upがNetworkDelayを
            // 引いて負になり、0比較では止まらなくなる
            || (self.tm.search_end != 0 && self.tm.search_end <= elapsed)
        {
            self.shared.aborted_search.store(true, Ordering::Relaxed);
            self.shared.stop.store(true, Ordering::Relaxed);
        } else if self.tm.search_end == 0
            && self.tm.use_time_management()
            // ponder中に予算を使い切っていたなら、ponderhit後の最初の
            // 判定でここへ来る（S:5551-5558の条件1と2）
            && (elapsed > self.tm.maximum() || self.stop_on_ponderhit)
        {
            self.tm.set_search_end(elapsed, ponderhit_offset);
        }
    }

    #[inline]
    fn draw_value(&self) -> Value {
        // 手番別の引き分けスコア（G10。S:1008-1009、S:2468）。
        // rootと同じ手番なら+、相手番なら-にする
        let base = if self.pos.side_to_move() == self.root_color {
            self.draw_value_us
        } else {
            -self.draw_value_us
        };
        // 千日手PVへの固着を防ぐ±1の揺らぎ（S:785のvalue_draw）
        base + VALUE_DRAW + 1 - (self.nodes & 2) as Value
    }

    /// 生の静的評価をeval hash経由で得る。ヒットなら
    /// evaluate()の全計算を省き、ミスなら計算して格納する。詰み圏の値は
    /// 入らない（evaluateの出力域のみ）。correction history補正は呼び側。
    #[inline]
    fn eval_cached(&mut self, key: u64) -> Value {
        if let Some(v) = self.shared.eval_hash.probe(key) {
            return v;
        }
        let v = self.evaluator.evaluate(&self.pos);
        self.shared.eval_hash.store(key, v);
        v
    }

    /// 指し手をStackへ記録する（yaneuraou-search.cpp:2102-2120）。
    /// continuation historyの面は、このplyの王手の有無と手の種別で決まる。
    #[inline]
    fn set_current_move(&mut self, ply: usize, m: Move, capture: bool) {
        let in_check = self.stack[ply + STACK_OFFSET].in_check;
        let e = &mut self.stack[ply + STACK_OFFSET];
        e.current_move = m;
        e.cont_base = ContinuationHistory::base(in_check, capture, m.piece_after(), m.to());
        e.cont_corr_base = ContinuationCorrectionHistory::base(m);
        // 子の `correction_value` が引く2か所を先読みさせる。
        // 子から見た1手前の手がm、2手前・4手前がこのplyの1つ下と3つ下になる
        let (pc, to) = (m.piece_after(), m.to());
        for back in [1, 3] {
            let base = self.stack[ply + STACK_OFFSET - back].cont_corr_base;
            self.hist.corr_cont.prefetch(base, pc, to);
        }
    }

    /// 1手前から6手前までのcontinuation historyの面。
    /// 余白があるのでply 0でも境界検査は要らない。
    #[inline]
    fn cont_bases(&self, ply: usize) -> [usize; 6] {
        std::array::from_fn(|i| self.stack[ply + STACK_OFFSET - 1 - i].cont_base)
    }

    /// LMRのリダクション量（G2。yaneuraou-search.cpp:5148-5151）。
    /// 返る値は1024倍の固定小数である。
    ///
    /// 基礎値は深さと手数の表の積。そこから窓幅の比で引き、改善して
    /// いなければ基礎値の206/512を足し、定数1133を足す。窓が広い
    /// （root窓に近い）ほど削らない。
    #[inline]
    fn reduction(&self, improving: bool, depth: u32, move_count: u32, delta: Value) -> i32 {
        let t = self.reductions;
        let scale = i32::from(t[(depth as usize).min(REDUCTIONS_LEN - 1)])
            * i32::from(t[(move_count as usize).min(REDUCTIONS_LEN - 1)]);
        scale - delta * 585 / self.root_delta + i32::from(!improving) * scale * 206 / 512 + 1133
    }

    /// その手の履歴の強さ（G2。yaneuraou-search.cpp:3924-3932）。
    /// 取る手は取った駒の価値とcapture history、静かな手はmain historyの
    /// 2倍と1手前・2手前のcontinuation historyで測る。do_moveの前に呼ぶ
    #[inline]
    fn stat_score(&self, m: Move, cont: &[usize; 6]) -> i32 {
        let to = m.to();
        let pc = m.piece_after();
        let captured = if m.is_drop() {
            Piece::EMPTY
        } else {
            self.pos.piece_on(to)
        };
        if !captured.is_empty() {
            863 * himawari_core::piece_value(captured.piece_type()) / 128
                + self.hist.capture.get(pc, to, captured.piece_type())
        } else {
            2 * self.hist.main.get(self.pos.side_to_move(), m)
                + self.hist.cont.get(cont[0], pc, to)
                + self.hist.cont.get(cont[1], pc, to)
        }
    }

    /// 往復手（shuffling）か（G5。yaneuraou-search.cpp:813-838）。
    /// 2手前・4手前と行き先を交換しているだけの手は、singularの検証探索に
    /// かけない。参照実装の `#if STOCKFISH` の外側が将棋版で、50手ルールの
    /// 条件を落とし、代わりに駒打ちを除いている。取る手も往復手ではない
    fn is_shuffling(&self, m: Move, ply: usize) -> bool {
        // capture_stage(m) は将棋版では単なるcapture(m)（position.h:1345）
        if m.is_drop() || !self.pos.piece_on(m.to()).is_empty() {
            return false;
        }
        // null moveをまたぐと2手前・4手前の手が繋がらない。局面が動いて
        // いない手数が浅いうちも対象にしない
        if self.pos.plies_from_null() <= 6 || ply < 18 {
            return false;
        }
        let move2 = self.stack[ply + STACK_OFFSET - 2].current_move;
        let move4 = self.stack[ply + STACK_OFFSET - 4].current_move;
        // is_ok()は「fromとtoが違う」ことなので、特殊手の除外に対応する
        if move2.is_special() || move4.is_special() || move2.is_drop() || move4.is_drop() {
            return false;
        }
        m.from_sq() == move2.to() && move2.from_sq() == move4.to()
    }

    /// correction historyの6要素を重み付きで合成する。
    /// 出典はやねうら王の `correction_value()`（yaneuraou-search.cpp:724-737）。
    /// 131072で割る前の値を返す。LMRのリダクションもこの値を読む。
    #[inline]
    fn correction_value(&self, ply: usize) -> i32 {
        let (pcv, micv, bnpcv, wnpcv) = self.shared.hists.corr.probe(&self.pos);
        // 余白があるのでply 0でも境界検査は要らない。前方は初期値のMove::NONE
        let prev1 = self.stack[ply + STACK_OFFSET - 1].current_move;
        let cntcv = if prev1.is_special() {
            CORR_CONT_DEFAULT
        } else {
            let to = prev1.to();
            let pc = self.pos.piece_on(to);
            self.hist
                .corr_cont
                .get(self.stack[ply + STACK_OFFSET - 2].cont_corr_base, pc, to)
                + self
                    .hist
                    .corr_cont
                    .get(self.stack[ply + STACK_OFFSET - 4].cont_corr_base, pc, to)
        };
        CORR_W_PAWN * pcv
            + CORR_W_MINOR * micv
            + CORR_W_NON_PAWN * (wnpcv + bnpcv)
            + CORR_W_CONT * cntcv
    }

    /// 生の静的評価にcorrection historyの補正を加える。
    /// 出典はやねうら王の `to_corrected_static_eval()`
    /// （yaneuraou-search.cpp:744-746）。詰み圏に入らないようクランプする。
    #[inline]
    fn to_corrected_with(&self, raw: Value, cv: i32) -> Value {
        (raw + cv / CORR_DIVISOR).clamp(VALUE_MATED_IN_MAX_PLY + 1, VALUE_MATE_IN_MAX_PLY - 1)
    }

    /// correction historyの補正込みの静的評価。
    #[inline]
    fn to_corrected(&self, raw: Value, ply: usize) -> Value {
        self.to_corrected_with(raw, self.correction_value(ply))
    }

    /// 探索を打ち切ってよい終端か。千日手・優等局面・
    /// 最大手数の到達・入玉宣言勝ちの4つを、この順に調べる。成立したら
    /// その評価値を返す。
    ///
    /// 通常探索と静止探索で判定も順序も同じなので1つにまとめる。
    /// 最大手数は静止探索でも通常探索と同じ扱いにする
    /// （yaneuraou-search.cpp:4620）。入玉宣言勝ちは参照実装への追従で唯一、
    /// 位置を残した判定で、玉が敵陣外なら即falseで安い
    #[inline]
    fn terminal_value(&self, ply: usize) -> Option<Value> {
        match self.pos.repetition_state(ply) {
            Repetition::Draw => return Some(self.draw_value()),
            Repetition::Win => return Some(mate_in(ply)),
            Repetition::Lose => return Some(mated_in(ply)),
            Repetition::Superior => return Some(VALUE_SUPERIOR),
            Repetition::Inferior => return Some(-VALUE_SUPERIOR),
            Repetition::None => {}
        }
        if self.max_moves_to_draw > 0 && self.pos.game_ply() >= self.max_moves_to_draw {
            return Some(self.draw_value());
        }
        if self.pos.can_declare_win() {
            return Some(mate_in(ply));
        }
        None
    }

    /// 置換表を引き、Stackへヒットの有無とttPvを記録する（/// yaneuraou-search.cpp:2623, 2657）。カットの判定は `tt_cutoff` が行う。
    fn probe_tt(&mut self, key: u64, ply: usize, is_pv: bool, excluded: Move) -> TtInfo {
        let hit = self.shared.tt.probe(key);
        // 1手先のノードがhistoryの更新条件で読む（yaneuraou-search.cpp:2623）
        self.stack[ply + STACK_OFFSET].tt_hit = hit.is_some();
        // 置換表にPVとして記録された値か（yaneuraou-search.cpp:2657）。
        // 除外手つき探索は同じplyでsearchを呼び直すので、上書きしない
        if excluded == Move::NONE {
            self.stack[ply + STACK_OFFSET].tt_pv = is_pv || hit.as_ref().is_some_and(|d| d.pv);
        }
        let mut tt = TtInfo {
            hit,
            mv: Move::NONE,
            value: VALUE_NONE,
            // 置換表のdepth欄はゲタを外して扱う。ヒットしないときは参照実装の
            // DEPTH_NONE（tt.cpp:445）に合わせて -3 とする
            depth: -TT_DEPTH_OFFSET,
            bound: Bound::None,
        };
        if let Some(data) = &hit {
            if let Some(m) = self.pos.to_move(data.mv)
                && self.pos.pseudo_legal(m)
            {
                tt.mv = m;
            }
            tt.value = value_from_tt(data.value, ply);
            tt.depth = i32::from(data.depth) - TT_DEPTH_OFFSET;
            tt.bound = data.bound;
        }
        tt
    }

    /// TTカットの判定（yaneuraou-search.cpp:2700-2748）。
    /// カットするならその値を返す。除外手つき探索中はカットしない
    /// （probeは行い、eval再利用は可）。
    // NodeInfoを組むより前に判定する（静的評価を取る前にカットを返す）ので、
    // 判定が読む値だけを個別に受ける
    #[allow(clippy::too_many_arguments)]
    fn tt_cutoff(
        &mut self,
        tt: &TtInfo,
        ply: usize,
        depth: u32,
        alpha: Value,
        beta: Value,
        is_pv: bool,
        excluded: Move,
        prior_capture: Piece,
        prev_sq: Option<Square>,
    ) -> Option<Value> {
        if tt.hit.is_none() || !(excluded == Move::NONE && !is_pv && tt.depth >= depth as i32) {
            return None;
        }
        let usable = match tt.bound {
            Bound::Exact => true,
            Bound::Lower => tt.value >= beta,
            Bound::Upper => tt.value <= alpha,
            Bound::None => false,
        };
        if !usable {
            return None;
        }
        // TTカットでもオーダリングを更新する
        // （yaneuraou-search.cpp:2735-2748）
        if tt.mv != Move::NONE && tt.value >= beta {
            let tt_capture = !tt.mv.is_drop() && !self.pos.piece_on(tt.mv.to()).is_empty();
            if !tt_capture {
                self.update_quiet_histories(ply, tt.mv, (130 * depth as i32 - 71).min(1043));
            }
            // 1手前の早い静かな手への追加ペナルティ
            if let Some(prev_sq) = prev_sq
                && self.stack[ply + STACK_OFFSET - 1].move_count <= 4
                && prior_capture.is_empty()
            {
                let pc = self.pos.piece_on(prev_sq);
                self.update_continuation_histories(ply - 1, pc, prev_sq, -2142);
            }
        }
        Some(tt.value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::Evaluator;
    use crate::timeman::TimeOptions;
    use himawari_core::Position;

    /// eval hashあり/なしで探索した (総ノード数, 最善手) を返す。
    fn search_nodes_best(sfen: &str, depth: u32, eval_hash: bool) -> (u64, Move) {
        let pos = Position::from_sfen(sfen).unwrap();
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            aborted_search: AtomicBool::new(false),
            ponder: AtomicBool::new(false),
            ponderhit_offset: AtomicI64::new(0),
            nodes: AtomicU64::new(0),
            increase_depth: AtomicBool::new(true),
            best_move_changes: AtomicU64::new(0),
            tt: Tt::new(16),
            eval_hash: if eval_hash {
                EvalHash::new()
            } else {
                EvalHash::disabled()
            },
            hists: Arc::new(SharedHistories::new(1)),
        });
        let limits = Limits {
            depth,
            ..Limits::default()
        };
        let tm = TimeManager::new(
            &limits,
            pos.side_to_move(),
            pos.game_ply(),
            &TimeOptions::default(),
        );
        let mut worker = Worker::new(
            pos,
            shared,
            limits,
            tm,
            0,
            1,
            Evaluator::material(),
            Histories::default(),
        );
        let result = worker.iterate(&mut |_| {});
        (worker.nodes, result.best)
    }

    /// 機能検証: eval hashは探索を変えないはずである。
    /// 偽ヒットを除けばノード数と最善手が有無で一致する。
    #[test]
    fn eval_hash_does_not_change_search() {
        // 中盤・序盤の複数局面で照合する
        for &(sfen, depth) in &[
            (himawari_core::SFEN_STARTPOS, 7),
            (
                "1n1gk2nl/1r4g2/1sppppspp/L5p2/1p5P1/2P6/1PSPPPPSP/7R1/1N1GKG1NL w BLPbp 24",
                6,
            ),
            ("4k4/9/9/5N3/9/9/9/9/4K4 b G 1", 5),
        ] {
            let (n_on, best_on) = search_nodes_best(sfen, depth, true);
            let (n_off, best_off) = search_nodes_best(sfen, depth, false);
            assert_eq!(
                n_on, n_off,
                "ノード数がeval hash有無で不一致: {sfen} (on={n_on}, off={n_off})"
            );
            assert_eq!(
                best_on.to_usi(),
                best_off.to_usi(),
                "最善手がeval hash有無で不一致: {sfen}"
            );
        }
    }
}
