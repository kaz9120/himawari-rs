//! 時間管理。参照実装 `source/timeman.cpp` の移植。
//!
//! 上限を超えてもその場では止めない。秒単位で切り上げた終了時刻を
//! `search_end` に予約し、最小思考時間がその下支えをする。

use std::time::{Duration, Instant};

use himawari_core::Color;

/// 終局までにこれくらい自分が指すと考えて計画を練る（timeman.cpp:17）。
/// 近年は終局までの平均手数が伸びているので160に設定されている
const MOVE_HORIZON: i64 = 160;

/// USI_Ponder有効時にoptimumへ足す分の除数（timeman.cpp:285-286）。
/// ponderhitすると時間が予測より余っていくので、思考時間を多めに取る。
/// 参照実装は `Stochastic_Ponder` が無効であることも条件にするが、
/// 本エンジンはそのオプションを持たないので常に無効として扱う
const PONDER_OPTIMUM_DIV: i64 = 4;

/// 思考時間の伸縮（yaneuraou-search.cpp:1958-1996）。
/// optimumを5係数の積で伸ばし縮めする。以前の3係数（falling /
/// stability / effort）はこのうち3つに対応するので、置き換えである。
///
/// fallingEval（S:1972-1976）は評価の下落で時間を伸ばす。入力は2本で、
/// 前回goの最善手の移動平均と4回前の反復のスコアである。前回goが
/// ないときは番兵の `VALUE_INFINITE` が入り、上限に張り付く。参照実装が
/// 「対局の初手にかかる時間に影響する」と書く意図的な挙動である
const FALLING_BASE: f64 = 11.325;
const FALLING_PREV_GO: f64 = 2.115;
const FALLING_ITER: f64 = 0.987;
const FALLING_MIN: f64 = 0.5688;
const FALLING_MAX: f64 = 1.5698;
/// timeReduction（S:1981-1983）は最善手が長く不変なら時間を縮める
/// ロジスティック。中心は「最善手が最後に変わった深さ + 11.57」
const TIME_REDUCTION_K: f64 = 0.5189;
const TIME_REDUCTION_CENTER: f64 = 11.57;
const TIME_REDUCTION_BASE: f64 = 0.723;
const TIME_REDUCTION_NUM: f64 = 0.79;
const TIME_REDUCTION_DEN: f64 = 1.104;
/// reduction（S:1984-1985）は前回goのtimeReductionを持ち越して均す
const REDUCTION_BASE: f64 = 1.455;
const REDUCTION_SCALE: f64 = 2.2375;
/// bestMoveInstability（S:1986）は最善手の入れ替わりで時間を伸ばす。
/// 分母はスレッド数で、スレッド横断のbestMoveChangesを平均する
const INSTABILITY_BASE: f64 = 1.04;
const INSTABILITY_COEF: f64 = 1.8956;
/// highBestMoveEffort（S:1969-1970, 1987）は最善手にノードが集中して
/// いれば時間を縮める。閾値の2値で、10万分率で92425（92.4%）以上
const EFFORT_MIN_DEPTH: u32 = 10;
const EFFORT_THRESHOLD: u64 = 92_425;
const EFFORT_SCALE: f64 = 0.666;
/// 合法手が1手しかないときの上限[ms]（S:1995-1996）
const SINGLE_MOVE_CAP_MS: f64 = 502.0;
/// increaseDepthの判定比（S:2050-2051）。totalTimeのこの割合を超えて
/// いれば、次の反復では実効深さを削って掘り直す
const INCREASE_DEPTH_RATIO: f64 = 0.503;
/// 持ち時間に合わせて定数を縮める。1手の配分がこれ未満なら、
/// 秒単位の切り上げをやめる。秒単位の時計を前提にした仕組みだからである
const ROUND_UP_MIN_AVAIL: i64 = 5000;
/// 最小思考時間は配分のこの分の1までに抑える
const MIN_THINK_AVAIL_DIV: i64 = 5;
/// NetworkDelayは配分のこの分の1までに抑える
const NETWORK_DELAY_AVAIL_DIV: i64 = 4;
/// NetworkDelay2は残り時間のこの分の1までに抑える
const RESERVE_TIME_DIV: i64 = 8;
/// NetworkDelay2を縮めるときの下限[ms]。指し手の返答の遅れを受け止める
const RESERVE_MIN_MS: i64 = 50;

/// 反復の終わりに時間管理へ渡す探索の統計（S:1961-2053の入力）。
/// 5係数はすべてここから決まる。
pub struct IterationStats {
    /// 最善手にノードがどれだけ集中しているか（10万分率）。
    /// 分子も分母もgo全体の累計である（S:1969-1970）
    pub nodes_effort: u64,
    /// 直近のroot探索の返り値（S:1438）。窓を外した回も含む
    pub best_value: i32,
    /// 前回goの最善手の `average_score`（S:1972-1976）。番兵の
    /// `VALUE_INFINITE` が入るとfallingEvalが上限に張り付く
    pub prev_go_average: i32,
    /// 4反復前のスコア（S:1972-1976, 2055-2056）
    pub iter_value: i32,
    /// 最善手が最後に変わった深さ（S:1426）。timeReductionの中心になる
    pub last_best_move_depth: u32,
    /// 確定した最後のイテレーションの深さ
    pub completed_depth: u32,
    /// 前回goのtimeReduction（S:1984-1985）
    pub prev_time_reduction: f64,
    /// 全スレッドの最善手の入れ替わり回数（S:1952-1956）
    pub tot_best_move_changes: f64,
    /// ワーカーの総数。bestMoveInstabilityの分母になる
    pub thread_count: usize,
    /// root手が1手しかないか（S:1995-1996）
    pub single_move: bool,
}

/// 反復の終わりに決まる時間の判断（S:1961-2053）。
pub struct IterationPlan {
    /// 予算を使い切ったか。真なら探索を終える
    pub budget_spent: bool,
    /// 深さを1段上げる余裕があるか（S:2049-2051）。偽なら次の反復では
    /// 実効深さを削って掘り直す。ponder中かどうかは呼び出し側が足す
    pub has_depth_slack: bool,
    /// 今回のtimeReduction（S:1981-1983）。次のgoへ持ち越す
    pub time_reduction: f64,
    /// 判定に使った経過時間[ms]。終了時刻の予約が読む
    pub elapsed: i64,
}

/// goコマンドの探索制限。時間はミリ秒。
#[derive(Clone, Default, Debug)]
pub struct Limits {
    /// go受領時刻（search.h:209、usi.cpp:506）。GUIとの時差をなくすため、
    /// goコマンドを受け取った直後に記録する。Noneなら計時の起点は
    /// `TimeManager::new` の呼び出し時刻になる
    pub start: Option<Instant>,
    pub btime: u64,
    pub wtime: u64,
    pub byoyomi: u64,
    pub binc: u64,
    pub winc: u64,
    pub movetime: u64,
    pub depth: u32,
    pub nodes: u64,
    pub infinite: bool,
}

impl Limits {
    /// 時間制御を行うか（search.h:195）。思考時間固定・深さ指定・
    /// ノード指定・無制限のときは時間制御に意味がないのでやらない。
    /// 参照実装の `mate` と `perft` に対応する制限は本エンジンにない
    #[inline]
    pub fn use_time_management(&self) -> bool {
        !(self.movetime > 0 || self.depth > 0 || self.nodes > 0 || self.infinite)
    }
}

/// 時間管理が読むエンジンオプション（timeman.cpp:26-57）。
#[derive(Clone, Copy, Debug)]
pub struct TimeOptions {
    /// 指し手がGUIへ届くまでの平均遅延[ms]（timeman.cpp:39）
    pub network_delay: i64,
    /// 最大遅延[ms]。切れ負けの瞬間だけこの分早く指す（timeman.cpp:44）
    pub network_delay2: i64,
    /// 最小思考時間[ms]（timeman.cpp:47）
    pub minimum_thinking_time: i64,
    /// 序盤重視率。百分率で、optimumの係数として働く（timeman.cpp:52）
    pub slow_mover: i64,
    /// 秒未満を切り上げるか（timeman.cpp:55）
    pub round_up_to_full_second: bool,
    /// 引き分けになる手数。0（指定なし）は100000として扱う
    /// （yaneuraou-search.cpp:72-77）
    pub max_moves_to_draw: i64,
    /// USI_Ponderが有効か（timeman.cpp:285）。optimumを1.25倍する条件で、
    /// 当該探索がponder探索かどうかでは判定しない
    pub ponder: bool,
}

/// 既定値はUSIオプションの既定（`EngineOptions::default`）から作る。
/// 既定値を書く場所を1か所にするためである
impl Default for TimeOptions {
    fn default() -> Self {
        crate::thread::EngineOptions::default().time_options()
    }
}

pub struct TimeManager {
    /// 計時の起点。`Limits::start`（go受領時刻）をコピーする（timeman.cpp:76）
    start: Instant,
    /// 時間制御を行うか（search.h:195）。falseなら時刻での停止をしない
    use_time_management: bool,
    /// 探索終了予定時刻。startからの経過時間[ms]で、0なら未確定
    /// （timeman.h:93）
    pub search_end: i64,
    /// 秒読みに入り持ち時間を使い切るべきか（timeman.h:117）
    is_final_push: bool,
    minimum_time: i64,
    optimum_time: i64,
    maximum_time: i64,
    minimum_thinking_time: i64,
    network_delay: i64,
    /// 今回の最大残り時間。これを超えてはならない（timeman.cpp:154-160）
    remain_time: i64,
    round_up_to_full_second: bool,
}

/// 持ち時間に合わせて縮めたあとの定数。
struct ScaledFloors {
    minimum_thinking_time: i64,
    network_delay: i64,
    network_delay2: i64,
    round_up_to_full_second: bool,
}

/// 実戦の持ち時間を前提にした定数を、短い持ち時間へ合わせて縮める。
///
/// 定数のままだと10+0.1では最初の5手で持ち時間の9割を使い、残り約1.2秒を
/// 使わないまま以後の全手が100msになる。秒読みは秒単位の時計を前提に毎手
/// 使い切る方式なので触らない。floodgateの300+10は配分が常に10秒以上、残りが
/// 常に10秒以上なので、どの定数も縮まない
fn scale_floors(opts: &TimeOptions, my_time: i64, byoyomi: i64, avail: i64) -> ScaledFloors {
    if byoyomi > 0 {
        return ScaledFloors {
            minimum_thinking_time: opts.minimum_thinking_time,
            network_delay: opts.network_delay,
            network_delay2: opts.network_delay2,
            round_up_to_full_second: opts.round_up_to_full_second,
        };
    }
    ScaledFloors {
        minimum_thinking_time: opts
            .minimum_thinking_time
            .min((avail / MIN_THINK_AVAIL_DIV).max(1)),
        network_delay: opts.network_delay.min(avail / NETWORK_DELAY_AVAIL_DIV),
        network_delay2: opts
            .network_delay2
            .min((my_time / RESERVE_TIME_DIV).max(RESERVE_MIN_MS)),
        round_up_to_full_second: opts.round_up_to_full_second && avail >= ROUND_UP_MIN_AVAIL,
    }
}

impl TimeManager {
    pub fn new(limits: &Limits, us: Color, game_ply: u16, opts: &TimeOptions) -> Self {
        let mut tm = TimeManager {
            // go受領時刻を起点にする（timeman.cpp:148）。指定がなければ現在時刻
            start: limits.start.unwrap_or_else(Instant::now),
            use_time_management: limits.use_time_management(),
            search_end: 0,
            is_final_push: false,
            minimum_time: 0,
            optimum_time: 0,
            maximum_time: 0,
            minimum_thinking_time: opts.minimum_thinking_time,
            network_delay: opts.network_delay,
            remain_time: 0,
            round_up_to_full_second: opts.round_up_to_full_second,
        };
        if limits.infinite || limits.depth > 0 || limits.nodes > 0 {
            // 時刻で止めない経路。ヘルパースレッドとgo ponderもここを通る
            return tm;
        }
        tm.init(limits, us, game_ply, opts);
        tm
    }

    /// 今回の思考時間を計算する（timeman.cpp:94-308）。
    fn init(&mut self, limits: &Limits, us: Color, game_ply: u16, opts: &TimeOptions) {
        let (my_time, inc) = match us {
            Color::Black => (limits.btime, limits.binc),
            Color::White => (limits.wtime, limits.winc),
        };
        let my_time = my_time as i64;
        let inc = inc as i64;
        let byoyomi = limits.byoyomi as i64;

        // 配分の分母は現行のまま据え置く。
        //
        // 参照実装の分母（move horizon方式のMTG）を入れた版は、支えを揃えた
        // うえでも730局で-15.7だった。分母だけをmove horizon方式にした
        // 版は-107.2で、支えを
        // 足しても中立までしか戻らない。分母は本エンジンの現行式が釣り合って
        // いるという判断で、支え（最小思考時間・秒単位切り上げ・停止の予約）
        // だけを参照実装から採る
        let rem_moves = (48i64 - i64::from(game_ply) / 2).max(16);
        let avail = my_time / rem_moves + byoyomi + inc;

        // 定数を持ち時間へ合わせて縮める。movetimeは残り時間を
        // 読まないので、ここで縮めても結果は変わらない
        let floors = scale_floors(opts, my_time, byoyomi, avail);
        self.minimum_thinking_time = floors.minimum_thinking_time;
        self.network_delay = floors.network_delay;
        self.round_up_to_full_second = floors.round_up_to_full_second;

        // 今回の最大残り時間（timeman.cpp:154-160）。秒読みは残り時間へ
        // 加算して考える。加算時間はこの指し手のあとに増えるので足さない。
        // 0にすると時間切れのあと自爆するので下限を置く
        self.remain_time = my_time + byoyomi - floors.network_delay2;
        self.remain_time = self
            .remain_time
            .max(if self.round_up_to_full_second { 100 } else { 1 });

        // 時間固定モード（timeman.cpp:189-193）
        if limits.movetime > 0 {
            let t = limits.movetime as i64;
            self.remain_time = t;
            self.minimum_time = t;
            self.optimum_time = t;
            self.maximum_time = t;
            return;
        }

        // 切れ負けであるか（timeman.cpp:196）
        let time_forfeit = inc == 0 && byoyomi == 0;

        // 対局長の見積もり（timeman.cpp:203-208）。切れ負けなら40手ぶん
        // 長く見る。序盤は定跡で進むので大きめに、40手目以降は減らして考える
        let ply = i64::from(game_ply);
        let move_horizon = if time_forfeit {
            MOVE_HORIZON + 40 - ply.min(40)
        } else {
            // + 20は調整項
            MOVE_HORIZON + 20 - ply.min(80)
        };

        // 残りの自分の手番の回数（timeman.cpp:213）。平手の初期局面はply==1。
        // ply == 255 or 256でMTGが1になるように2を足す
        let mtg = (opts.max_moves_to_draw - ply + 2).min(move_horizon) / 2;

        if mtg <= 0 {
            // 終局までの最大手数が指定されている前提なので通らないはず。
            // 事故防止のために何か設定はしておく（timeman.cpp:215-222）
            self.minimum_time = 500;
            self.optimum_time = 500;
            self.maximum_time = 500;
            return;
        }
        if mtg == 1 {
            // この手番で終了なので使いきれば良い（timeman.cpp:223-228）
            self.minimum_time = self.remain_time;
            self.optimum_time = self.remain_time;
            self.maximum_time = self.remain_time;
            return;
        }

        // 最小思考時間（timeman.cpp:235-236）。秒未満を切り上げないときは
        // 秒未満での戦いなので下限を1にする
        self.minimum_time = (self.minimum_thinking_time - self.network_delay).max(
            if self.round_up_to_full_second {
                1000
            } else {
                1
            },
        );

        // 最適・最大思考時間には、まず上限値を入れておく（timeman.cpp:239）
        self.optimum_time = self.remain_time;
        self.maximum_time = self.remain_time;

        // optimumの候補。最小思考時間の床は参照実装から採る（timeman.cpp:257）
        let t1 = self.minimum_time + avail;

        // maximumの候補。倍率は現行の3倍を据え置く。
        // 切れ負けでは5分を切ったら比率を抑える（timeman.cpp:270-276）。
        // これは分母に依らない安全弁なので採る
        let mut max_ratio = 3.0f64;
        if time_forfeit {
            max_ratio = max_ratio.min((my_time as f64 / (60.0 * 1000.0)).max(1.0));
        }
        let t2 = self.minimum_time + (avail as f64 * max_ratio) as i64;

        // slowMoverは百分率で、optimumの係数として働く（timeman.cpp:280-281）。
        //
        // optimumからもNetworkDelayを引く。参照実装は引かないが、あちらは
        // `minimumTime = MinimumThinkingTime - NetworkDelay` の床（既定1880ms）が
        // optimumへ加算されるため、その中に遅延の保護が含まれている。床を下げると
        // 保護まで消える。10+0.1で `MinimumThinkingTime=1` にして測ったところ、
        // optimumが常に121ms大きくなり、availが42〜600msしかない条件では1.6〜2.8倍の
        // 下駄になった。終盤の到達深さが8ply落ちる
        self.optimum_time =
            (t1.min(self.optimum_time) * opts.slow_mover / 100 - self.network_delay).max(1);
        self.maximum_time = t2.min(self.maximum_time);

        // USI_Ponder有効時はoptimumを1.25倍する（timeman.cpp:285-286）。
        // ponderhitすると、その手の思考の一部が相手の時計で進むぶん、
        // 自分の持ち時間は予測より余っていく。maximumは触らない。
        // 参照実装は倍率をNetworkDelayを引く前に掛けるが、本エンジンは
        // optimumからもNetworkDelayを引くので引いたあとに掛ける
        if opts.ponder {
            self.optimum_time += self.optimum_time / PONDER_OPTIMUM_DIV;
        }

        // 秒読みモードで持ち時間がないなら、使いきったほうが得
        // （timeman.cpp:291-302）。持ち時間が秒読みの1.2倍未満なら該当する
        self.is_final_push = false;
        if byoyomi > 0 && my_time < (byoyomi as f64 * 1.2) as i64 {
            let t = byoyomi + my_time;
            self.minimum_time = t;
            self.optimum_time = t;
            self.maximum_time = t;
            // "ponderhit"の時刻から数えてminimum分は使ってほしい
            self.is_final_push = true;
        }

        // 残り時間 - NetworkDelay2よりは短くしないと切れ負けになりうる
        // （timeman.cpp:305-307）
        self.minimum_time = self
            .round_up(self.minimum_time)
            .min(self.remain_time)
            .max(0);
        self.optimum_time = self.optimum_time.min(self.remain_time);
        self.maximum_time = self.round_up(self.maximum_time).min(self.remain_time);
        // maximumがoptimumを下回らないようにする（初期の時間管理から引き継ぐ安全弁）。
        // 参照実装はこの保証を持たないが、床を下げると32%の手でmaximumのほうが
        // 小さくなり、optimumが目標として働かなくなる
        self.maximum_time = self.maximum_time.max(self.optimum_time);
    }

    /// 1秒単位で繰り上げてdelayを引く（timeman.cpp:312-344）。
    /// `remain_time` よりは小さくなるように制限する
    fn round_up(&self, t0: i64) -> i64 {
        if self.round_up_to_full_second {
            // 1000で繰り上げる。MinimumThinkingTimeが最低値
            let mut t = (((t0 + 999) / 1000) * 1000).max(self.minimum_thinking_time);
            t -= self.network_delay;
            // 元の値より小さいなら、もう1秒使わないともったいない
            if t < t0 {
                t += 1000;
            }
            t.min(self.remain_time)
        } else {
            let mut t = t0.max(self.minimum_thinking_time);
            t -= self.network_delay;
            t.min(self.remain_time)
        }
    }

    /// 探索の終了が確定したので、秒単位で切り上げた時刻を予約する
    /// （timeman.cpp:347-373）。
    ///
    /// 引数 `e` はstartからの経過時間[ms]。呼び出し側が既に持っている
    /// 値を渡してもらい、二度測るのを避ける。
    /// 引数 `ponderhit_offset` は `ponderhitTime - startTime`[ms]
    /// （timeman.h:120）。"ponderhit" を受けるまでは0で、goからの経過時間と
    /// ponderhitからの経過時間が一致する。参照実装はTimeManagementの
    /// メンバーだが、本エンジンでは書き手がUSIスレッドで読み手が探索
    /// スレッドなので、共有の原子変数から読んだ値を渡してもらう。
    /// ponderhitからの経過時間で切り上げつつ、goから数えて `minimum()`
    /// の分は思考させる。`is_final_push` のときは切り上げの基準も
    /// ponderhitの時刻から数える
    pub fn set_search_end(&mut self, e: i64, ponderhit_offset: i64) {
        // 1. ponderhitからの経過時間（go ponderしていないならgoからの経過）
        let t1 = e - ponderhit_offset;
        // 2. goした時刻からminimum()を足し、ponderhitからの経過へ換算した値
        let t2 = if self.is_final_push {
            self.minimum_time
        } else {
            self.minimum_time - ponderhit_offset
        };
        // 大きいほうを秒単位で切り上げ、startからの経過時間へ戻す
        self.search_end = self.round_up(t1.max(t2)) + ponderhit_offset;
    }

    /// 次の反復を回す時間があるか（yaneuraou-search.cpp:1961-2053）。
    /// optimumを5係数の積で伸ばし縮めし、経過時間と比べる。
    ///
    /// 停止条件を満たしてもここでは止めない。呼び出し側がponderの有無を
    /// 見て、終了時刻を予約するか予約だけ控えるかを決める。
    pub fn plan_next_iteration(&self, s: &IterationStats) -> IterationPlan {
        // 評価が下がっているほど伸ばす（S:1972-1976）
        let falling = ((FALLING_BASE
            + FALLING_PREV_GO * f64::from(s.prev_go_average - s.best_value)
            + FALLING_ITER * f64::from(s.iter_value - s.best_value))
            / 100.0)
            .clamp(FALLING_MIN, FALLING_MAX);
        // 最善手が長く不変なら縮める（S:1980-1983）
        let center = f64::from(s.last_best_move_depth) + TIME_REDUCTION_CENTER;
        let time_reduction = TIME_REDUCTION_BASE
            + TIME_REDUCTION_NUM
                / (TIME_REDUCTION_DEN
                    + (-TIME_REDUCTION_K * (f64::from(s.completed_depth) - center)).exp());
        // 前回goのtimeReductionで均す（S:1984-1985）
        let reduction =
            (REDUCTION_BASE + s.prev_time_reduction) / (REDUCTION_SCALE * time_reduction);
        // 最善手が揺れているほど伸ばす（S:1986）
        let instability =
            INSTABILITY_BASE + INSTABILITY_COEF * s.tot_best_move_changes / s.thread_count as f64;
        // ノードが集中しているなら縮める（S:1987）
        let high_effort =
            if s.completed_depth >= EFFORT_MIN_DEPTH && s.nodes_effort >= EFFORT_THRESHOLD {
                EFFORT_SCALE
            } else {
                1.0
            };
        let mut total_time =
            self.optimum() as f64 * falling * reduction * instability * high_effort;
        // 合法手が1手ならこれ以上考えても仕方がない（S:1995-1996）
        if s.single_move {
            total_time = total_time.min(SINGLE_MOVE_CAP_MS);
        }
        let elapsed = self.elapsed_ms();
        IterationPlan {
            budget_spent: (elapsed as f64) > total_time.min(self.maximum() as f64),
            // 深さを1段上げる余裕はないが、まだ時間はある状態（S:2049-2051）
            has_depth_slack: (elapsed as f64) <= total_time * INCREASE_DEPTH_RATIO,
            time_reduction,
            elapsed,
        }
    }

    #[inline]
    pub fn use_time_management(&self) -> bool {
        self.use_time_management
    }

    #[inline]
    pub fn minimum(&self) -> i64 {
        self.minimum_time
    }

    #[inline]
    pub fn optimum(&self) -> i64 {
        self.optimum_time
    }

    #[inline]
    pub fn maximum(&self) -> i64 {
        self.maximum_time
    }

    #[inline]
    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }

    /// startからの経過時間[ms]（timeman.h:58）。
    #[inline]
    pub fn elapsed_ms(&self) -> i64 {
        self.start.elapsed().as_millis() as i64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byoyomi_only() {
        let limits = Limits {
            byoyomi: 3000,
            ..Limits::default()
        };
        let tm = TimeManager::new(&limits, Color::Black, 50, &TimeOptions::default());
        // 残り時間0でも秒読み分は使える。ただしNetworkDelay2の1120msは
        // 切れ負け防止に残すので、remain_timeは3000-1120=1880になる
        assert_eq!(tm.remain_time, 1880);
        // 持ち時間0は秒読みの1.2倍未満なのでisFinalPushが立ち、
        // 3手すべてが byoyomi + my_time = 3000 になる（T:291-302）。
        // そのあとremain_timeで頭打ちされて1880で揃う（T:305-307）
        assert!(tm.is_final_push);
        assert_eq!(tm.minimum(), 1880);
        assert_eq!(tm.optimum(), 1880);
        assert_eq!(tm.maximum(), 1880);
    }

    #[test]
    fn fischer_adds_minimum_to_current_formula() {
        // 300+10。配分の分母は現行式を据え置き、最小思考時間の
        // 床だけを参照実装から採る
        let limits = Limits {
            btime: 300_000,
            binc: 10_000,
            ..Limits::default()
        };
        let tm = TimeManager::new(&limits, Color::Black, 1, &TimeOptions::default());
        assert_eq!(tm.remain_time, 300_000 - 1120);
        // rem_moves = max(48 - 0, 16) = 48、avail = 300000/48 + 10000 = 16250
        // optimum = minimumTime + avail - NetworkDelay = 1880 + 16250 - 120
        assert_eq!(tm.optimum(), 18010);
        // maximum = round_up(1880 + 16250*3) = round_up(50630)
        assert_eq!(tm.maximum(), 50_880);
        assert_eq!(tm.minimum(), 1880);
    }

    #[test]
    fn slow_mover_scales_optimum() {
        let limits = Limits {
            btime: 300_000,
            binc: 10_000,
            ..Limits::default()
        };
        let opts = TimeOptions {
            slow_mover: 200,
            ..TimeOptions::default()
        };
        let tm = TimeManager::new(&limits, Color::Black, 1, &opts);
        // optimumだけが2倍になる（T:280）。NetworkDelayは倍率のあとに引く
        assert_eq!(tm.optimum(), 18130 * 2 - 120);
        assert_eq!(tm.maximum(), 50_880);
    }

    #[test]
    fn ponder_scales_optimum() {
        // USI_Ponder有効時はoptimumだけが1.25倍になる（T:285-286）
        let limits = Limits {
            btime: 300_000,
            binc: 10_000,
            ..Limits::default()
        };
        let opts = TimeOptions {
            ponder: true,
            ..TimeOptions::default()
        };
        let tm = TimeManager::new(&limits, Color::Black, 1, &opts);
        assert_eq!(tm.optimum(), 18010 + 18010 / 4);
        assert_eq!(tm.maximum(), 50_880);
        assert_eq!(tm.minimum(), 1880);
    }

    #[test]
    fn sudden_death_caps_max_ratio() {
        // 切れ負け（加算も秒読みもない）で残り2分。max_ratioが2に下がる
        let limits = Limits {
            btime: 120_000,
            ..Limits::default()
        };
        let tm = TimeManager::new(&limits, Color::Black, 1, &TimeOptions::default());
        // avail = 120000/48 = 2500で5秒未満なので定数が縮む。
        // 最小思考時間500、NetworkDelay120で、minimumTime = 380
        // optimum = 380 + 2500 - 120
        assert_eq!(tm.optimum(), 380 + 2500 - 120);
        // maximum = 380 + 2500*2 から同じ120を引く。切り上げはしない
        assert_eq!(tm.maximum(), 380 + 5000 - 120);
    }

    #[test]
    fn floodgate_floors_stay_constant() {
        // 300+10の最も切迫した場面（持ち時間0で10秒の加算だけ）でも、
        // 定数はどれも縮まない
        let limits = Limits {
            btime: 10_000,
            binc: 10_000,
            ..Limits::default()
        };
        let tm = TimeManager::new(&limits, Color::Black, 150, &TimeOptions::default());
        assert_eq!(tm.remain_time, 10_000 - 1120);
        assert_eq!(tm.minimum_thinking_time, 2000);
        assert_eq!(tm.network_delay, 120);
        assert!(tm.round_up_to_full_second);
    }

    #[test]
    fn short_clock_scales_floors() {
        // 10+0.1の開始局面。定数のままなら最小思考時間だけで1.88秒になる
        let limits = Limits {
            btime: 10_000,
            binc: 100,
            ..Limits::default()
        };
        let tm = TimeManager::new(&limits, Color::Black, 24, &TimeOptions::default());
        // avail = 10000/36 + 100 = 377。最小思考時間75、NetworkDelay94
        assert_eq!(tm.minimum_thinking_time, 75);
        assert_eq!(tm.network_delay, 94);
        assert!(!tm.round_up_to_full_second);
        // 残り時間は10000/8 = 1250がNetworkDelay2より大きいので1120のまま
        assert_eq!(tm.remain_time, 10_000 - 1120);
        // optimum = max(75 - 94, 1) + 377 - 94 = 284
        assert_eq!(tm.optimum(), 284);
    }

    #[test]
    fn short_clock_shrinks_reserve() {
        // 残りが少なくなったら、取り置きも一緒に縮めて使い切らないようにする
        let limits = Limits {
            btime: 1000,
            binc: 100,
            ..Limits::default()
        };
        let tm = TimeManager::new(&limits, Color::Black, 80, &TimeOptions::default());
        assert_eq!(tm.remain_time, 1000 - 125);
        // 取り置きには下限がある
        let limits = Limits {
            btime: 200,
            binc: 100,
            ..Limits::default()
        };
        let tm = TimeManager::new(&limits, Color::Black, 80, &TimeOptions::default());
        assert_eq!(tm.remain_time, 200 - 50);
    }

    #[test]
    fn byoyomi_keeps_floors() {
        // 秒読みは秒単位の時計を前提にするので、定数を縮めない
        let limits = Limits {
            byoyomi: 1000,
            ..Limits::default()
        };
        let tm = TimeManager::new(&limits, Color::Black, 80, &TimeOptions::default());
        assert_eq!(tm.minimum_thinking_time, 2000);
        assert_eq!(tm.network_delay, 120);
        assert!(tm.round_up_to_full_second);
        assert_eq!(tm.remain_time, 100);
    }

    #[test]
    fn infinite_has_no_limits() {
        let limits = Limits {
            infinite: true,
            ..Limits::default()
        };
        let tm = TimeManager::new(&limits, Color::Black, 1, &TimeOptions::default());
        // 無制限では時刻による停止をしない
        assert!(!tm.use_time_management());
    }

    #[test]
    fn movetime_is_fixed() {
        let limits = Limits {
            movetime: 1000,
            ..Limits::default()
        };
        let tm = TimeManager::new(&limits, Color::White, 1, &TimeOptions::default());
        // 参照実装はmovetimeからNetworkDelayを引かない（T:189-193）。
        // 超過の判定はcheck_timeがlimits.movetimeと直接比べる
        assert_eq!(tm.optimum(), 1000);
        assert_eq!(tm.maximum(), 1000);
        assert_eq!(tm.minimum(), 1000);
        assert!(!tm.use_time_management());
    }

    #[test]
    fn round_up_lifts_to_full_second() {
        let limits = Limits {
            btime: 300_000,
            binc: 10_000,
            ..Limits::default()
        };
        let tm = TimeManager::new(&limits, Color::Black, 1, &TimeOptions::default());
        // 1200ms → 2000msへ繰り上げ、NetworkDelayの120msを引く
        assert_eq!(tm.round_up(1200), 1880);
        // 2000ms → 2000ms、120msを引くと元より小さいのでもう1秒使う
        assert_eq!(tm.round_up(2000), 2880);
        // MinimumThinkingTimeが下限として働く
        assert_eq!(tm.round_up(10), 1880);
    }

    #[test]
    fn search_end_respects_minimum() {
        let limits = Limits {
            btime: 300_000,
            binc: 10_000,
            ..Limits::default()
        };
        let mut tm = TimeManager::new(&limits, Color::Black, 1, &TimeOptions::default());
        assert_eq!(tm.search_end, 0);
        // 経過時間が最小思考時間より短くても、最小思考時間まで予約する
        tm.set_search_end(300, 0);
        assert_eq!(tm.search_end, 1880);
    }

    #[test]
    fn search_end_counts_from_ponderhit() {
        // go ponderで5秒読んだあとponderhitした場合（T:347-373）。
        // 切り上げの基準はponderhitからの経過時間になる
        let limits = Limits {
            btime: 300_000,
            binc: 10_000,
            ..Limits::default()
        };
        let mut tm = TimeManager::new(&limits, Color::Black, 1, &TimeOptions::default());
        // t1 = 6000 - 5000 = 1000、t2 = 1880 - 5000 = -3120。
        // 大きいほうの1000を切り上げて1880、そこへoffsetを戻す
        tm.set_search_end(6000, 5000);
        assert_eq!(tm.search_end, 1880 + 5000);
    }
}
