//! 反復深化とroot局面の探索。
//!
//! aspiration windowで反復を回し、MultiPVの各ラインをinfoで報告する。
//! 前処理ツールが使う局面の差し替えと、静止局面までの前進もここに置く。

#[allow(clippy::wildcard_imports)]
use super::*;

impl Worker {
    /// MultiPV>1のときだけライン番号を出す（現行互換）。
    #[inline]
    pub(super) fn multipv_label(&self, pv_idx: usize) -> usize {
        if self.multi_pv > 1 { pv_idx + 1 } else { 0 }
    }

    /// 反復深化1周分の報告を組み立てる。3か所で
    /// seldepth・nodes・elapsed_ms・hashfullの採り方が同じなのでまとめる。
    /// nodesは全ワーカー合算（単スレッドではローカル値と一致）。
    pub(super) fn iter_info(
        &self,
        depth: u32,
        multipv: usize,
        score: Value,
        pv: Vec<Move>,
    ) -> IterInfo {
        IterInfo {
            depth,
            seldepth: self.sel_depth.max(depth),
            multipv,
            score,
            pv,
            nodes: self.shared.nodes.load(Ordering::Relaxed).max(self.nodes),
            elapsed_ms: self.tm.elapsed().as_millis() as u64,
            hashfull: self.shared.tt.hashfull(),
        }
    }

    /// aspirationのfail時に途中経過を報告する。
    /// PVは並べ替え済みの `root_moves[pv_idx]` から採る。
    pub(super) fn report_bound(
        &self,
        on_info: &mut dyn FnMut(SearchInfo),
        depth: u32,
        pv_idx: usize,
        score: Value,
        bound: ScoreBound,
    ) {
        let line: Vec<Move> = self.root_moves[pv_idx].pv.clone();
        if line.is_empty() {
            return;
        }
        on_info(SearchInfo::Bound(
            self.iter_info(depth, self.multipv_label(pv_idx), score, line),
            bound,
        ));
    }

    /// root手をスコアの降順に安定ソートする（search.h:168-171）。
    /// 同点なら前の反復のスコアの降順で、それも同点なら元の並びを保つ。
    pub(super) fn sort_root_moves(moves: &mut [RootMove]) {
        moves.sort_by(|a, b| b.score.cmp(&a.score).then(b.prev_score.cmp(&a.prev_score)));
    }

    /// 局面を差し替え、plyごとの状態を初期化する。
    ///
    /// 前処理ツールがWorkerを使い回すために要る。局面ごとに `new` を呼ぶと
    /// history一式（数MB）を毎回確保することになり、実測で679局面/秒まで
    /// 落ちた。**スタックを戻すのは結果を処理順に依存させないためである。**
    /// historyは持ち越すが、これは同じ対局を続けて読むときと同じ状況で、
    /// 手順の再現性を壊さない。
    pub fn set_position(&mut self, pos: Position) {
        self.root_color = pos.side_to_move();
        self.pos = SearchBoard::new(pos);
        // accumulatorのスタックを捨てる。前の局面の計算済みフラグが
        // 残っていると、全く別の局面の評価値をそのまま読んでしまう
        self.evaluator.new_search(&self.pos);
        self.stack.fill(STACK_INIT);
        self.sel_depth = 0;
        self.root_delta = 1;
        self.root_depth = 0;
        self.nmp_min_ply = 0;
        self.last_iteration_pv.clear();
    }

    /// 静止探索を1回走らせ、到達した静止局面まで `self.pos` を進める。
    /// 戻り値は進めた手数である。
    ///
    /// 教師局面をqsearchのPV葉へ置き換える前処理が使う。**探索の内側は
    /// 変えない。** qsearchは最善手を置換表へ書くので、置換表を辿れば
    /// PVを再構成できる。qsearchにPV収集を足すと、全ての葉で費用を払う
    /// ことになるため採らない。
    ///
    /// 打ち切りは `max_plies` で行う。置換表の衝突で手が繋がり続ける場合と、
    /// 千日手のような循環に備える。
    pub fn walk_to_quiet(&mut self, max_plies: usize) -> usize {
        let mut plies = 0;
        while plies < max_plies {
            self.stack[STACK_OFFSET].in_check = self.pos.in_check();
            self.qsearch::<true>(-VALUE_INFINITE, VALUE_INFINITE, 0);
            let key = self.pos.key();
            let Some(data) = self.shared.tt.probe(key) else {
                break;
            };
            let Some(m) = self.pos.to_move(data.mv) else {
                break;
            };
            if !self.pos.pseudo_legal(m) || !self.pos.is_legal(m) {
                break;
            }
            self.make_move(m);
            plies += 1;
        }
        plies
    }

    /// 反復深化。各イテレーション完了時にon_iterを呼ぶ。
    pub fn iterate(&mut self, on_info: &mut dyn FnMut(SearchInfo)) -> SearchResult {
        // 入玉宣言勝ち: 成立していれば探索せず宣言する
        if self.pos.can_declare_win() {
            return SearchResult {
                best: Move::WIN,
                score: mate_in(0),
                ponder: Move::NONE,
                root_score: -VALUE_INFINITE,
                root_average_score: -VALUE_INFINITE,
                pv: Vec::new(),
                completed_depth: 0,
            };
        }
        self.shared.tt.new_search();
        self.evaluator.new_search(&self.pos);
        // lowPly historyはgoのたびに埋め直す（S:1539-1540）
        self.hist.new_search();
        // 前回のgoで得たPVは今回の探索では使えないので捨てる
        // （yaneuraou-search.cpp:943）
        self.last_iteration_pv.clear();
        let mut list = MoveList::default();
        self.pos.generate_legal(false, &mut list);
        self.root_moves = list
            .as_slice()
            .iter()
            .map(|&mv| RootMove {
                mv,
                score: -VALUE_INFINITE,
                prev_score: VALUE_ZERO,
                pv: Vec::new(),
                effort: 0,
                average_score: -VALUE_INFINITE,
                mean_squared_score: MEAN_SQUARED_NONE,
            })
            .collect();
        if self.root_moves.is_empty() {
            return SearchResult {
                best: Move::RESIGN,
                score: mated_in(0),
                ponder: Move::NONE,
                root_score: -VALUE_INFINITE,
                root_average_score: -VALUE_INFINITE,
                pv: Vec::new(),
                completed_depth: 0,
            };
        }
        // 前回のgoと手番が入れ替わっているなら、持ち越したスコアの符号を
        // 反転させる（S:1483-1492）。番兵はそのまま残す
        let root_game_ply = self.pos.game_ply();
        if (i32::from(self.memory.last_game_ply) - i32::from(root_game_ply)) & 1 != 0 {
            if self.memory.best_previous_score != VALUE_INFINITE {
                self.memory.best_previous_score = -self.memory.best_previous_score;
            }
            if self.memory.best_previous_average_score != VALUE_INFINITE {
                self.memory.best_previous_average_score = -self.memory.best_previous_average_score;
            }
        }
        let mut last_score = VALUE_ZERO;
        // 最後に確定した最善手のPVとスコア（S:1426-1428）。中断した反復の
        // 詰み負けスコアはここへ戻すために覚える
        let mut last_best_pv: Vec<Move> = Vec::new();
        let mut last_best_score = -VALUE_INFINITE;
        // 最後に出したinfoが未確定の窓外れ（lowerbound / upperbound）か。
        // 打ち切りでこのまま終わると、GUIやCSAブリッジは確定していない値を
        // その手のスコアとして記録する。実際にfloodgateで `4723++` という
        // 値が残り、直後に評価が8300も反転した
        let mut unresolved_bound = false;
        // 確定した最後のイテレーションの深さ。
        let mut completed_depth = 0u32;
        // メインワーカーか（S:1917）。時間管理と統計はメインだけが行う
        let is_main = self.thread_idx == 0;
        // 直近のroot探索の返り値（S:1438）。窓を外した回も含む。
        // 思考時間のfallingEvalが読む
        let mut best_value = -VALUE_INFINITE;
        // 最善手が最後に変わった深さ（S:1426）。timeReductionの中心になる
        let mut last_best_move_depth = 0u32;
        // 直近4反復のスコアの環状バッファ（S:1440, 2055-2056）。
        // fallingEvalが4回前の反復のスコアと比べる
        let mut iter_value = [VALUE_ZERO; 4];
        let mut iter_idx = 0usize;
        // 最善手が入れ替わった回数の統計（S:1439）。反復ごとに半減させ、
        // 全スレッドの計数を足し込む。読むのは時間管理だけなので、
        // 更新も時間管理の節で行う
        let mut tot_best_move_changes = 0.0f64;
        // 今回goのtimeReduction（S:1439, 2062）。次のgoへ持ち越す
        let mut time_reduction = 1.0f64;
        // 同じ深さを掘り直した回数（S:1531-1534）。実効深さを削る量に効く
        let mut search_again_counter = 0u32;
        // 前回goのスコアで環状バッファを埋める（S:1495-1498）。
        // 前回がなければ0で埋める
        iter_value.fill(if self.memory.best_previous_score == VALUE_INFINITE {
            VALUE_ZERO
        } else {
            self.memory.best_previous_score
        });
        let max_depth = if self.limits.depth > 0 {
            self.limits.depth
        } else {
            (MAX_PLY - 1) as u32
        };

        'deepening: for depth in 1..=max_depth {
            // ヘルパーは反復を深さでずらす
            if self.skip_iteration(depth) {
                continue;
            }
            for rm in &mut self.root_moves {
                rm.prev_score = rm.score;
            }
            // 深さが増えていないなら同じ深さを掘り直したことになる
            // （S:1600-1606）。メインが余り時間から立てた旗を全スレッドが読む
            if !self.shared.increase_depth.load(Ordering::Relaxed) {
                search_again_counter += 1;
            }
            // seldepthはイテレーションごとに測り直す
            self.sel_depth = 0;
            // singularの多段化のマージンが読む（S:1550, 3779）。実効深さ
            // （fail highで削った値）ではなく反復深化の深さを入れる
            self.root_depth = depth;
            let lines = self.multi_pv.min(self.root_moves.len());
            // 直前ラインの出力スコア。頭打ちの基準に使う
            let mut prev_line_score = VALUE_INFINITE;
            for pv_idx in 0..lines {
                // ラインごとのaspiration（S:1669-1673）。窓幅は評価値の
                // 二乗平均に比例して広がり、中心はスコアの移動平均に置く。
                // 深さ1では二乗平均が番兵のままなので窓が全開になる
                let mut delta = ASPIRATION_BASE()
                    + (self.thread_idx % ASPIRATION_THREAD_SPREAD) as Value
                    + self.root_moves[pv_idx].mean_squared_score.abs() / ASPIRATION_MSS_DIV();
                let avg = self.root_moves[pv_idx].average_score;
                let mut alpha = (avg - delta).max(-VALUE_INFINITE);
                let mut beta = (avg + delta).min(VALUE_INFINITE);
                // fail highした回数。1回ごとに実効深さを1段削る（S:1705-1706）
                let mut failed_high_cnt = 0u32;
                loop {
                    // fail highと掘り直しの分だけ実効深さを削る（S:1699-1707）。
                    // searchAgain 4回につき1回は深さが進むようにしてある
                    let adjusted_depth = depth
                        .saturating_sub(failed_high_cnt)
                        .saturating_sub(3 * (search_again_counter + 1) / 4)
                        .max(1);
                    let score = self.search_root(adjusted_depth, alpha, beta, pv_idx, on_info);
                    // 窓を外した回も含めて毎回並べ替える（S:1717）。
                    // 実の値を持つのは1手目とalphaを更新した手だけなので、
                    // 安定ソートでなければ残りの並びが崩れる
                    Self::sort_root_moves(&mut self.root_moves[pv_idx..]);
                    best_value = score;
                    if self.stopped() {
                        break 'deepening;
                    }
                    if score <= alpha {
                        // fail low: 実際の評価はこの値以下。
                        // 窓を広げて読み直す前に、途中経過として報告する
                        self.report_bound(on_info, depth, pv_idx, score, ScoreBound::Upper);
                        if pv_idx == 0 {
                            unresolved_bound = true;
                        }
                        // 窓は下へずらす。上端は元のalphaに畳む（S:1777-1784）
                        beta = alpha;
                        alpha = (score - delta).max(-VALUE_INFINITE);
                        failed_high_cnt = 0;
                        // 評価が下がったので、予約した停止を解除する
                        // （S:1783-1784）。読み直す価値のある局面である
                        self.stop_on_ponderhit = false;
                    } else if score >= beta {
                        // fail high: 実際の評価はこの値以上
                        self.report_bound(on_info, depth, pv_idx, score, ScoreBound::Lower);
                        if pv_idx == 0 {
                            unresolved_bound = true;
                        }
                        // 下端は元のbetaから幅の分だけ戻した位置まで上げる
                        // （S:1786-1790）
                        alpha = (beta - delta).max(alpha);
                        beta = (score + delta).min(VALUE_INFINITE);
                        failed_high_cnt += 1;
                    } else {
                        // 成功。最善手は並べ替えでこのラインの先頭に来ている。
                        // 出力スコアを前のラインで頭打ちにする。fail-softでは
                        // ラインごとにaspiration窓が違い、返り値が窓に依存する
                        // ため、探索順のままだと後のラインが前を上回ることが
                        // ある。root_movesへは生の値を残し、出力だけ整える。
                        // 並べ替えで整えると、確定済みラインと手が重複する。
                        // 基準は前ラインの生スコアではなく出力スコアにする。
                        // 生スコアだと、前ラインが頭打ちされた分を後ろのラインが
                        // 飛び越えられる（s1=100・s2=150・s3=120で3本目が
                        // 120となり2本目の100を上回る）
                        let line_score = score.min(prev_line_score);
                        prev_line_score = line_score;
                        let line_pv = self.root_moves[pv_idx].pv.clone();
                        if pv_idx == 0 {
                            last_score = score;
                            unresolved_bound = false;
                        }
                        on_info(SearchInfo::Iteration(self.iter_info(
                            depth,
                            self.multipv_label(pv_idx),
                            line_score,
                            line_pv,
                        )));
                        break;
                    }
                    // 外したので次は幅を4/3倍にする（S:1795）
                    delta += delta / ASPIRATION_GROWTH_DIV();
                }
            }
            // 今回の反復のPVを覚える（yaneuraou-search.cpp:1846-1853）。
            // 次の反復のfollow_pv判定が読む。打ち切られた反復のPVは
            // 途中までしか探索していないので採らない
            if !self.shared.stop.load(Ordering::Relaxed) {
                completed_depth = depth;
                // 最善手が前の反復から変わった深さを覚える（S:1848-1851）。
                // timeReductionのロジスティックの中心になる
                if self.last_iteration_pv.is_empty()
                    || self.last_iteration_pv.first() != self.root_moves[0].pv.first()
                {
                    last_best_move_depth = depth;
                }
                self.last_iteration_pv.clone_from(&self.root_moves[0].pv);
            }
            // 最善手が変わったら、確定したPVとスコアを覚え直す（S:1888-1893）
            if self.root_moves[0].pv.first() != last_best_pv.first() {
                last_best_pv.clone_from(&self.root_moves[0].pv);
                last_best_score = self.root_moves[0].score;
                last_best_move_depth = depth;
            }
            // ここまで来れば深さ1は完走している。以降はstopに従う
            self.depth1_done = true;
            // 詰みが確定したら打ち切る。反復深化なので、より短い
            // 詰みがあれば浅い周で見つかっている。これ以上読んでも結論は
            // 変わらない。詰まされる側も同じで、より長く粘る手があれば
            // alpha-betaが既にそちらを選んでいる
            if self.multi_pv == 1 && last_score.abs() >= VALUE_MATE_IN_MAX_PLY {
                break;
            }
            if self.stopped() {
                break;
            }
            if self.limits.nodes > 0 && self.nodes >= self.limits.nodes {
                break;
            }
            // ここから先はメインだけが行う（S:1917）
            if !is_main {
                continue;
            }
            if self.tm.use_time_management() {
                // 反復の世代が進んだので、最善手の入れ替わりの重みを半分にし、
                // 全スレッドの今回の入れ替わり回数を汲み出す（S:1577-1581,
                // 1952-1956）。メインだけが集計する。
                //
                // 下の測り直しの条件より外に置く。stop_on_ponderhitは
                // fail lowで下りるので、測り直さない反復の後にまた測る
                // ことがある。その反復でも半減を飛ばさないためである
                tot_best_move_changes /= 2.0;
                tot_best_move_changes +=
                    self.shared.best_move_changes.swap(0, Ordering::Relaxed) as f64;
            }
            // 次の反復を回す時間があるか（S:1961-2053）。停止条件を満たして
            // もその場では止めず、秒単位で切り上げた終了時刻を予約する。
            // 予約済みなら終了は確定しているので測り直さない
            if self.tm.use_time_management()
                && !self.shared.stop.load(Ordering::Relaxed)
                && !self.stop_on_ponderhit
                && self.tm.search_end == 0
            {
                let stats = IterationStats {
                    // 最善手にノードがどれだけ集中しているか（10万分率）。
                    // 分子も分母もgo全体の累計である（S:1969-1970）
                    nodes_effort: self.root_moves[0].effort * 100_000 / self.nodes.max(1),
                    best_value,
                    prev_go_average: self.memory.best_previous_average_score,
                    iter_value: iter_value[iter_idx],
                    last_best_move_depth,
                    completed_depth,
                    prev_time_reduction: self.memory.previous_time_reduction,
                    tot_best_move_changes,
                    thread_count: self.thread_count,
                    single_move: self.root_moves.len() == 1,
                    nodes: self.nodes,
                };
                let plan = self.tm.plan_next_iteration(&stats);
                time_reduction = plan.time_reduction;
                if plan.budget_spent {
                    if self.shared.ponder.load(Ordering::SeqCst) {
                        // go ponder中はGUIの指示があるまで止められない。
                        // 停止を予約だけしておく（S:2043-2044）
                        self.stop_on_ponderhit = true;
                    } else {
                        let ponderhit_offset = self.shared.ponderhit_offset.load(Ordering::SeqCst);
                        self.tm.set_search_end(plan.elapsed, ponderhit_offset);
                    }
                } else {
                    // 深さを1段上げる余裕がないなら、次の反復では実効深さを
                    // 削って掘り直す（S:2049-2051）。ponder中は掘り直さない
                    self.shared.increase_depth.store(
                        self.shared.ponder.load(Ordering::SeqCst) || plan.has_depth_slack,
                        Ordering::Relaxed,
                    );
                }
            }
            // 4反復前のスコアと比べるための環状バッファ（S:2055-2056）
            iter_value[iter_idx] = best_value;
            iter_idx = (iter_idx + 1) & 3;
        }
        // 今回goのtimeReductionを次のgoへ持ち越す（S:2062）
        self.memory.previous_time_reduction = time_reduction;
        // 時間管理の計画と実績を残す。切れ負けの調査で「計画が過大だったのか、
        // 計画の外で遅れたのか」を分けるために要る。GUIのログに残る
        if is_main && self.tm.use_time_management() {
            on_info(SearchInfo::Note(format!(
                "time plan: minimum {} optimum {} maximum {} end {} elapsed {} ponderhit {} stop {}",
                self.tm.minimum(),
                self.tm.optimum(),
                self.tm.maximum(),
                self.tm.search_end,
                self.tm.elapsed_ms(),
                self.shared.ponderhit_offset.load(Ordering::SeqCst),
                self.shared.stop.load(Ordering::Relaxed),
            )));
        }
        // 中断した探索で得た詰み負けのスコアは信用できない（S:1864-1887）。
        // 残りのroot手を読めば、負けが延びたり反証されたりしうる。前の
        // 反復で確定したPVとスコアへ戻す。
        //
        // 参照実装はこの判定を反復の末尾で毎回行う。本エンジンは中断時に
        // 反復の途中で抜けるので、ループを出たあとに1回だけ行う。
        // abortedSearchが立つとstopも立ち、その反復で必ずループを出るので
        // 判定の回数は変わらない
        if self.shared.aborted_search.load(Ordering::Relaxed)
            && self.root_moves[0].score != -VALUE_INFINITE
            && self.root_moves[0].score <= VALUE_MATED_IN_MAX_PLY
            && !last_best_pv.is_empty()
        {
            // 確定した最善手を先頭へ移す
            if let Some(i) = self
                .root_moves
                .iter()
                .position(|rm| rm.mv == last_best_pv[0])
            {
                let rm = self.root_moves.remove(i);
                self.root_moves.insert(0, rm);
            }
            self.root_moves[0].pv.clone_from(&last_best_pv);
            self.root_moves[0].score = last_best_score;
            last_score = last_best_score;
        }
        // 未確定の窓外れで終わるなら、確定した最後の結果を出し直す。
        // これを出さないと、消費側の最後の1行が lowerbound / upperbound の
        // ままになり、指し手と食い違うスコアがその手の評価として残る
        if unresolved_bound && completed_depth > 0 && !self.root_moves[0].pv.is_empty() {
            on_info(SearchInfo::Iteration(self.iter_info(
                completed_depth,
                self.multipv_label(0),
                last_score,
                self.root_moves[0].pv.clone(),
            )));
        }
        // 次のgoで使う値を持ち越す（S:1249-1253）。参照実装はbest thread
        // のrootMoves[0]から採る。投票で他スレッドが選ばれたときは、
        // 呼び出し側（ThreadPool）がそちらの値で上書きする
        self.memory.best_previous_score = self.root_moves[0].score;
        self.memory.best_previous_average_score = self.root_moves[0].average_score;
        self.memory.last_game_ply = root_game_ply;
        // check_limitsで2048刻みに流し込んだ分を除いた端数を合算する
        self.shared
            .nodes
            .fetch_add(self.nodes % 2048, Ordering::Relaxed);
        SearchResult {
            best: self.root_moves[0].mv,
            score: last_score,
            ponder: self.root_moves[0].pv.get(1).copied().unwrap_or(Move::NONE),
            root_score: self.root_moves[0].score,
            root_average_score: self.root_moves[0].average_score,
            pv: self.root_moves[0].pv.clone(),
            completed_depth,
        }
    }

    /// root_moves[pv_idx..]を探索する（上位の確定済みラインは除外）。
    /// 戻り値はfail-softのスコア。各root手のスコアとPVは `root_moves` へ
    /// 直接書く（S:4113-4165）。呼び出し側は探索のあとで並べ替えて先頭を読む
    pub(super) fn search_root(
        &mut self,
        depth: u32,
        mut alpha: Value,
        beta: Value,
        pv_idx: usize,
        on_info: &mut dyn FnMut(SearchInfo),
    ) -> Value {
        // リダクションの窓幅項の基準（yaneuraou-search.cpp:1708）
        self.root_delta = beta - alpha;
        let mut best = -VALUE_INFINITE;
        let moves: Vec<Move> = self.root_moves[pv_idx..].iter().map(|rm| rm.mv).collect();
        // 子が読むcontinuation historyの面の選択に要る
        let in_check = self.pos.in_check();
        self.stack[STACK_OFFSET].in_check = in_check;
        // 1手目のノードがhistoryの更新条件でrootのStackを読む
        // （yaneuraou-search.cpp:2623, 3126-3133）。参照実装はrootも同じ
        // search()なので、置換表ヒットと静的評価がrootでも埋まっている
        let key = self.pos.key();
        self.stack[STACK_OFFSET].tt_hit = self.shared.tt.probe(key).is_some();
        // rootは常にPVノードなのでttPvはtrue（yaneuraou-search.cpp:2657）
        self.stack[STACK_OFFSET].tt_pv = true;
        self.stack[STACK_OFFSET + 2].cutoff_cnt = 0;
        self.stack[STACK_OFFSET].stat_score = 0;
        // rootは常に前回PVの上にいる（yaneuraou-search.cpp:2370）
        self.stack[STACK_OFFSET].follow_pv = true;
        // continuation historyの面（1手前から6手前まで）。rootでも
        // statScoreの計算に要る
        let cont = self.cont_bases(0);
        self.stack[STACK_OFFSET].static_eval = if in_check {
            VALUE_NONE
        } else {
            let raw = self.eval_cached(key);
            self.to_corrected(raw, 0)
        };
        for (j, &m) in moves.iter().enumerate() {
            let i = pv_idx + j;
            self.stack[STACK_OFFSET].move_count = (j + 1) as u32;
            // 長考中だけ「今読んでいる手」を出す。短い探索で
            // 出すとinfo行が溢れる
            if self.tm.elapsed().as_millis() as u64 >= CURRMOVE_MIN_MS {
                on_info(SearchInfo::CurrMove { depth, mv: m });
            }
            let capture = !m.is_drop() && !self.pos.piece_on(m.to()).is_empty();
            // 1手目のノードがhistoryの更新量で読む
            // （yaneuraou-search.cpp:3924-3932）
            self.stack[STACK_OFFSET].stat_score = self.stat_score(m, &cont);
            self.set_current_move(0, m, capture);
            let nodes_before = self.nodes;
            self.make_move(m);
            let mut child_pv = Vec::new();
            // rootはcut_node = false。PVの第1手と再探索はfalse、
            // 第2手以降のnull windowは反転してtrueになる
            let value = if j == 0 {
                -self.search::<true>(-beta, -alpha, depth - 1, 1, m, &mut child_pv, false)
            } else {
                let v =
                    -self.search::<false>(-alpha - 1, -alpha, depth - 1, 1, m, &mut child_pv, true);
                if v > alpha && !self.stopped() {
                    -self.search::<true>(-beta, -alpha, depth - 1, 1, m, &mut child_pv, false)
                } else {
                    v
                }
            };
            self.unmake_move(m);
            // 打ち切られた分もこの手の探索コストなので先に計上
            self.root_moves[i].effort += self.nodes - nodes_before;
            if self.stopped() {
                return best;
            }
            // スコアの移動平均と二乗平均（yaneuraou-search.cpp:4105-4110）。
            // 次の反復のaspiration窓の中心と幅になる。番兵のときは初回として
            // 値をそのまま入れる。二乗平均は符号を保つため |value| を掛ける
            {
                let rm = &mut self.root_moves[i];
                rm.average_score = if rm.average_score != -VALUE_INFINITE {
                    (value + rm.average_score) / 2
                } else {
                    value
                };
                rm.mean_squared_score = if rm.mean_squared_score != MEAN_SQUARED_NONE {
                    (value * value.abs() + rm.mean_squared_score) / 2
                } else {
                    value * value.abs()
                };
            }
            // root手のスコアとPVを書く（S:4113-4165）。実の値を持つのは
            // 1手目とalphaを更新した手だけで、残りは -VALUE_INFINITE にする。
            // この形なら途中で打ち切っても並べ替えが前の深さの結論を壊さない
            if j == 0 || value > alpha {
                self.root_moves[i].score = value;
                let line = &mut self.root_moves[i].pv;
                line.clear();
                line.push(m);
                line.extend_from_slice(&child_pv);
                // 最善手が入れ替わった回数を数える
                // （yaneuraou-search.cpp:4157-4160）。1手目での確定は
                // 入れ替わりではない。MultiPVでは1本目だけを数える
                if j > 0 && pv_idx == 0 {
                    self.shared
                        .best_move_changes
                        .fetch_add(1, Ordering::Relaxed);
                }
            } else {
                self.root_moves[i].score = -VALUE_INFINITE;
            }
            if value > best {
                best = value;
                if value > alpha {
                    if value >= beta {
                        // 参照実装はrootも同じ経路を通る
                        // （yaneuraou-search.cpp:4214）
                        self.stack[STACK_OFFSET].cutoff_cnt += 1;
                        break;
                    }
                    alpha = value;
                }
            }
        }
        best
    }
}
