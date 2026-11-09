"""NNUE trainer v2."""

import argparse
import csv
import math
import multiprocessing
import os
import sys
import time

import torch
import torch.nn.functional as F
from torch.utils.data import DataLoader
from torch.utils.tensorboard import SummaryWriter

from model import ARCH, EFF_OUT, EFFECT_INPUTS, NnueModel, loss_fn
from optim import MaskedAdam
from dataset import PsvBatchLoader, PsvDataset, RankLoader, collate_psv
from quantize import save_hmwr


def lr_lambda(step, warmup_steps, total_steps, min_lr, peak_lr):
    if step < warmup_steps:
        return step / max(warmup_steps, 1)
    progress = (step - warmup_steps) / max(total_steps - warmup_steps, 1)
    ratio = min_lr / peak_lr
    return ratio + (1.0 - ratio) * 0.5 * (1.0 + math.cos(math.pi * progress))


def validate(model, valid_loader, device):
    model.eval()
    total_loss = 0.0
    total_n = 0
    with torch.no_grad():
        for batch in valid_loader:
            if batch is None:
                continue
            stm_i, stm_o, opp_i, opp_o, targets = [
                x.to(device) for x in batch[:5]
            ]
            # 利きの入力特徴。抽出させていなければ空で来る。
            # 古い拡張のPsvDatasetは5本しか返さないので、そのときは渡さない
            eff = [x.to(device) for x in batch[7:9]]
            out = model(stm_i, stm_o, opp_i, opp_o, *eff)
            loss = loss_fn(out, targets)
            total_loss += loss.item() * targets.size(0)
            total_n += targets.size(0)
    model.train()
    return total_loss / max(total_n, 1)


def main():
    p = argparse.ArgumentParser(description="NNUE trainer v2")
    p.add_argument("--data", help="Training PSV file")
    p.add_argument("--valid", help="Validation PSV file")
    p.add_argument("--out", help="Output .hmwr path（--eval-only 以外では必須）")
    p.add_argument("--epochs", type=int, default=1)
    p.add_argument(
        "--positions",
        type=int,
        help="学習データの先頭のこの件数だけを使う。--batch-loader のときだけ効く",
    )
    p.add_argument("--batch", type=int, default=16384)
    p.add_argument("--peak-lr", type=float, default=1e-3)
    p.add_argument("--min-lr", type=float, default=1e-6)
    p.add_argument("--warmup-steps", type=int, default=100)
    p.add_argument("--lambda", type=float, default=0.7, dest="lambda_")
    p.add_argument(
        "--rank-data",
        help="兄弟局面のランキング群（psv rankの出力）",
    )
    p.add_argument(
        "--rank-batch", type=int, default=4096,
        help="1ステップに引くランキング群の数",
    )
    p.add_argument(
        "--rank-alpha", type=float, default=1.0,
        help="ランキング損失の重み",
    )
    p.add_argument(
        "--rank-margin", type=float, default=0.02,
        help="ヒンジのマージン（シグモイド空間）",
    )
    p.add_argument("--score-limit", type=int, default=0,
                   help="この絶対値以上の評価値を持つ局面を学習から除外する（0=無効）")
    p.add_argument("--score-clamp", type=int, default=0,
                   help="評価値をこの絶対値へ丸めてから教師信号にする（0=無効）")
    p.add_argument("--workers", type=int, default=4)
    p.add_argument("--log-interval", type=int, default=100)
    p.add_argument("--valid-interval", type=int, default=2000)
    p.add_argument("--patience", type=int, default=0, help="Early stopping patience (0=disabled)")
    p.add_argument("--log-file", help="TSV log output path")
    p.add_argument("--log-dir", help="TensorBoard log directory")
    p.add_argument("--checkpoint-dir", help="Checkpoint directory")
    p.add_argument("--resume", help="Resume from checkpoint")
    p.add_argument("--registry", help="Experiment registry TSV path")
    p.add_argument("--name", default="", help="Experiment name for registry")
    p.add_argument("--notes", default="", help="Notes for registry")
    p.add_argument("--device", default="cpu")
    p.add_argument(
        "--effect-inputs",
        dest="effect_inputs",
        choices=list(EFFECT_INPUTS),
        help="利きのaccumulatorへ特徴を立てるか。f2は駒×升×利き数の特徴を"
             "立てる処置群、noneは立てない統制群。利きの構成（e<E>）で省くとf2になる。"
             "それ以外の構成では渡せない",
    )
    p.add_argument(
        "--init-net",
        help="既存の.hmwrを初期値に読む。FTは常に読み、後段は形が一致する層だけ"
             "読む",
    )
    p.add_argument(
        "--freeze-ft",
        action="store_true",
        help="FTを更新しない。後段の候補を絞るときに使う。"
             "採用の判断には使わない",
    )
    p.add_argument(
        "--ft-clip",
        type=float,
        default=0.0,
        help="畳み込み後のFT重みをこの絶対値へ収める（0=無効）。"
             "i8で格納するとき飽和させないための制約で、量子化スケール127に"
             "対しては1.0を指定する",
    )
    p.add_argument(
        "--side-clip",
        type=float,
        default=None,
        help="側の表の重み（畳み込み後）とバイアスをこの絶対値へ収める（0=無効）。"
             "差分で持つ側のaccをi16に収めるための制約で、F2と最小の攻め駒の組では"
             "1.5で上限の見積もりが31,051になる。射影の間隔は --ft-clip-interval に従う。"
             "省くと、側のaccをi16で持つ f2_minatt の構成では1.5、それ以外では0になる",
    )
    p.add_argument(
        "--init-checkpoint",
        help="f32のチェックポイント（.pt）から重みだけを読んで新しい学習を始める"
             "。--resume と違い step・optimizer・schedulerは引き継がず、"
             "lrは渡した値から改めて減衰する。--init-net は量子化を経た.hmwrを読むので"
             "世代を重ねるほど丸め誤差が乗るが、こちらは乗らない",
    )
    p.add_argument(
        "--eval-only",
        action="store_true",
        help="学習せず、--init-net で読んだネットの検証損失だけを出す"
             "。教師データの条件を変えて同じネットを測るときに使う。"
             "読むのは量子化済みの.hmwrなので、学習中に出る値とは丸めのぶん違う",
    )
    p.add_argument(
        "--ft-clip-interval",
        type=int,
        default=50,
        help="FT重みの射影を何ステップおきに行うか。毎ステップは"
             "重すぎる。検証と書き出しの直前はこの設定に関係なく必ず射影する",
    )
    p.add_argument(
        "--seed",
        type=int,
        help="モデル初期化とデータ順序の乱数種。指定しないとPyTorchが実行ごとに"
             "違う種を引くため、同じ条件でもvalid lossが動く",
    )
    p.add_argument(
        "--mmap",
        action="store_true",
        help="学習データをmmapで開く（RAMに載らない規模用。速度は落ちる）",
    )
    p.add_argument(
        "--batch-loader",
        action="store_true",
        help="バッチ一括抽出のローダを使う",
    )
    p.add_argument(
        "--factorized",
        action="store_true",
        help="学習時のみ駒単独の仮想特徴を併用する",
    )
    p.add_argument(
        "--dense-ft",
        action="store_true",
        help="FT勾配をdenseにする（SparseAdamを外し、MPSで学習できる）",
    )
    args = p.parse_args()

    # 利きのaccumulatorは構成で決まる。利きの構成で省いたら、
    # 現行の評価関数と同じF2の特徴を立てる。どちらの群かは来歴に残る（下の eff_tag）
    if EFF_OUT > 0 and args.effect_inputs is None:
        args.effect_inputs = "f2"
    if EFF_OUT == 0 and args.effect_inputs is not None:
        p.error(f"--effect-inputs は利きの構成（e<E>）でだけ使える。いまの構成は {ARCH}")
    eff_inputs = args.effect_inputs == "f2"
    # 側のaccをi16で持つ構成は、上限なしで学習すると読み込みで弾かれる。
    # 1日学習してから気づかないよう、省いたら現行の評価関数と同じ1.5をかける
    if args.side_clip is None:
        args.side_clip = 1.5 if "-f2_minatt" in ARCH else 0.0
    if EFF_OUT > 0 and not args.batch_loader:
        p.error("利きの構成は --batch-loader が要る（逐次のローダは特徴を作らない）")
    # 来歴に群を残す。統制と処置は同じ構成で、ファイルの寸法では見分けられない
    eff_tag = f" effect_inputs={args.effect_inputs}" if args.effect_inputs else ""

    if args.eval_only:
        # 測るだけなので教師データは要らない。読むネットと指標が要る
        if not args.init_net and not args.init_checkpoint:
            p.error("--eval-only には --init-net か --init-checkpoint が要る（測るネットがない）")
        if not args.valid:
            p.error("--eval-only には --valid が要る（指標がない）")
    elif not args.data:
        p.error("--data が要る")
    if not args.eval_only and not args.out:
        p.error("--out が要る")

    device = torch.device(args.device)
    print(f"Device: {device}", file=sys.stderr)

    # 種を固定しないと初期化とデータ順が実行ごとに変わり、条件の差と
    # 初期値の差を分けられない
    if args.seed is not None:
        torch.manual_seed(args.seed)
        print(f"Seed: {args.seed}", file=sys.stderr)

    if args.eval_only:
        train_loader = None
        data_n = 0
    elif args.batch_loader:
        train_loader = PsvBatchLoader(
            args.data, args.batch, lambda_=args.lambda_,
            score_limit=args.score_limit, mmap=args.mmap, shuffle=True,
            score_clamp=args.score_clamp, seed=args.seed or 0,
            positions=args.positions,
            effect_inputs=eff_inputs,
        )
        data_n = train_loader.n
    else:
        train_ds = PsvDataset(
            args.data, lambda_=args.lambda_, score_limit=args.score_limit,
            mmap=args.mmap, score_clamp=args.score_clamp,
        )
        # macOSのstart methodはspawnで、workerごとにデータセットがpickle複製
        # される。数十GB規模ではOOMになるためforkを明示し、CoWで共有する
        mp_ctx = multiprocessing.get_context("fork") if args.workers > 0 else None
        train_loader = DataLoader(
            train_ds, batch_size=args.batch, shuffle=True,
            num_workers=args.workers, collate_fn=collate_psv,
            pin_memory=(device.type != "cpu"),
            multiprocessing_context=mp_ctx,
        )
        data_n = len(train_ds)
    steps_per_epoch = math.ceil(data_n / args.batch)
    total_steps = args.epochs * steps_per_epoch

    valid_loader = None
    if args.valid:
        if args.batch_loader:
            # validには score_limit も score_clamp も適用しない。
            # 教師信号の作り方を変えると指標が変わり、条件間で
            # valid loss を比べられなくなる
            valid_loader = PsvBatchLoader(
                args.valid, args.batch, lambda_=args.lambda_, shuffle=False,
                effect_inputs=eff_inputs,
            )
            valid_n = valid_loader.n
        else:
            valid_ds = PsvDataset(args.valid, lambda_=args.lambda_)
            valid_loader = DataLoader(
                valid_ds, batch_size=args.batch, shuffle=False,
                num_workers=args.workers, collate_fn=collate_psv,
                multiprocessing_context=mp_ctx,
            )
            valid_n = len(valid_ds)
        print(f"検証データ: {valid_n}局面", file=sys.stderr)

    model = NnueModel(
        sparse_ft=not args.dense_ft,
        factorized=args.factorized,
        effect_inputs=args.effect_inputs,
    ).to(device)
    if args.effect_inputs is not None:
        print(
            f"利きの入力: {args.effect_inputs}（幅{EFF_OUT}、"
            f"{'特徴を立てる処置群' if eff_inputs else '特徴を立てない統制群'}）",
            file=sys.stderr,
        )

    if args.init_net and args.init_checkpoint:
        p.error("--init-net と --init-checkpoint は同時に使えない（初期値は1つ）")
    if args.init_checkpoint:
        # 重みだけ読む。stepとoptimizerを引き継がないので、lrは渡した値から
        # 改めて減衰する。前世代の続きを別のデータで学習するための経路である
        ckpt = torch.load(args.init_checkpoint, map_location=device,
                          weights_only=False)
        model.load_state_dict(ckpt["model"])
        print(f"初期値（f32）: {args.init_checkpoint} "
              f"step={ckpt.get('step', '?')} valid={ckpt.get('best_valid', '?')}",
              file=sys.stderr)
        if args.freeze_ft:
            model.ft.weight.requires_grad_(False)
            model.ft_bias.requires_grad_(False)
            if model.ft_p is not None:
                model.ft_p.weight.requires_grad_(False)
    elif args.init_net:
        from quantize import load_into
        print(f"初期値: {load_into(model, args.init_net, args.freeze_ft)}",
              file=sys.stderr)
    elif args.freeze_ft:
        p.error("--freeze-ft には --init-net か --init-checkpoint が要る（凍結する重みがない）")

    if args.eval_only:
        vl = validate(model, valid_loader, device)
        src = args.init_net or args.init_checkpoint
        print(f"{src}\t{args.valid}\t{vl:.5f}")
        return

    dense_params = [
        model.ft_bias,
        model.l2.weight, model.l2.bias,
        model.out.weight, model.out.bias,
    ]
    # 隠れ層は構成によって数が変わる
    if model.l3 is not None:
        dense_params.extend([model.l3.weight, model.l3.bias])
    # 利きのaccumulator。統制群の表は凍結してあり、下で外れる
    if model.eff is not None:
        dense_params.extend([model.eff.weight, model.eff_bias])
    ft_params = [model.ft.weight]
    if model.ft_p is not None:
        ft_params.append(model.ft_p.weight)
    # 凍結した重みは更新対象から外す。残すとoptimizerが空の勾配を扱う
    dense_params = [t for t in dense_params if t.requires_grad]
    ft_params = [t for t in ft_params if t.requires_grad]
    def lr_fn(step):
        return lr_lambda(step, args.warmup_steps, total_steps, args.min_lr, args.peak_lr)

    optimizer_dense = torch.optim.Adam(dense_params, lr=args.peak_lr)
    # FT勾配をdenseにするとMPSへ載せられる。更新則はSparseAdamと
    # 同じ「出現した行だけ動かす」を保つ
    # FTを凍結すると更新対象が空になる。optimizerは作らない
    optimizer_ft = (
        None
        if not ft_params
        else MaskedAdam(ft_params, lr=args.peak_lr)
        if args.dense_ft
        else torch.optim.SparseAdam(ft_params, lr=args.peak_lr)
    )
    scheduler_dense = torch.optim.lr_scheduler.LambdaLR(optimizer_dense, lr_lambda=lr_fn)
    scheduler_ft = (
        None
        if optimizer_ft is None
        else torch.optim.lr_scheduler.LambdaLR(optimizer_ft, lr_lambda=lr_fn)
    )

    step = 0
    start_epoch = 0
    best_valid = float("inf")
    best_step = 0
    no_improve = 0

    if args.resume:
        print(f"チェックポイント復元: {args.resume}", file=sys.stderr)
        ckpt = torch.load(args.resume, map_location=device, weights_only=False)
        model.load_state_dict(ckpt["model"])
        optimizer_dense.load_state_dict(ckpt["optimizer_dense"])
        if optimizer_ft is not None and ckpt["optimizer_ft"] is not None:
            optimizer_ft.load_state_dict(ckpt["optimizer_ft"])
        scheduler_dense.load_state_dict(ckpt["scheduler_dense"])
        if scheduler_ft is not None and ckpt["scheduler_ft"] is not None:
            scheduler_ft.load_state_dict(ckpt["scheduler_ft"])
        step = ckpt["step"]
        best_valid = ckpt.get("best_valid", float("inf"))
        best_step = ckpt.get("best_step", 0)
        # **エポック内の位置も戻す。** stepだけ戻して
        # エポックを頭から回すと、schedulerが復元した値から進むので
        # cosineの終端を越えて学習率が再上昇する。epochs=1が既定の
        # 本エンジンでは、この経路が常に壊れていた
        start_epoch = step // steps_per_epoch
        skip = step % steps_per_epoch
        if hasattr(train_loader, "skip_batches"):
            train_loader.epoch = start_epoch
            train_loader.skip_batches = skip
            print(
                f"再開位置: epoch {start_epoch} のバッチ {skip}/{steps_per_epoch} から",
                file=sys.stderr,
            )
        elif skip:
            print(
                f"警告: このローダはエポック途中からの再開に対応していない"
                f"（{skip}バッチぶんを重複して学習する）",
                file=sys.stderr,
            )

    print(
        f"学習データ: {data_n}局面 × {args.epochs}エポック, "
        f"batch={args.batch}, peak_lr={args.peak_lr}, "
        f"warmup={args.warmup_steps}, total_steps={total_steps}, "
        f"λ={args.lambda_}",
        file=sys.stderr,
    )

    writer = SummaryWriter(args.log_dir) if args.log_dir else None
    log_file = None
    if args.log_file:
        is_new = not os.path.exists(args.log_file)
        log_file = open(args.log_file, "a")
        if is_new:
            log_file.write("type\tstep\tepoch\tsamples\tloss\tlr\tsps\telapsed_s\n")

    if args.checkpoint_dir:
        os.makedirs(args.checkpoint_dir, exist_ok=True)

    t0 = time.time()
    t_log = time.time()
    loss_acc = 0.0
    loss_n = 0
    samples_log = 0
    rank_acc = 0.0
    rank_fire = 0.0
    rank_n = 0
    samples_done = step * args.batch
    early_stopped = False

    rank_loader = None
    if args.rank_data:
        rank_loader = RankLoader(
            args.rank_data, args.rank_batch, seed=args.seed, effect_inputs=eff_inputs,
        )
        print(f"ランキング群: {rank_loader.n}群 ({args.rank_data})", file=sys.stderr)

    model.train()
    for epoch in range(start_epoch, args.epochs):
        for batch in train_loader:
            if batch is None:
                continue

            # 6・7本目は指し手のラベルで、学習には使わない。8・9本目は
            # 利きの入力特徴。抽出させていなければ空で来る
            tensors = [x.to(device) for x in batch]
            stm_i, stm_o, opp_i, opp_o, targets = tensors[:5]
            eff_stm, eff_opp = tensors[7:9]
            n = targets.size(0)

            optimizer_dense.zero_grad()
            if optimizer_ft is not None:
                optimizer_ft.zero_grad()
            x = model.transform_both(stm_i, stm_o, opp_i, opp_o, eff_stm, eff_opp)
            out = model.value(x)
            # ログとvalidに載せるのは評価値の損失だけにする。合計を載せると
            # λを変えるたびに指標が動き、過去の学習と比べられなくなる
            loss = loss_fn(out, targets)
            value_loss = loss.detach()
            if rank_loader is not None:
                # 兄弟局面のランキング損失。正例の葉が負例の葉
                # より親視点で良い、をシグモイド空間のヒンジで課す
                r_si, r_so, r_oi, r_oo, r_par, r_es, r_eo = [
                    x.to(device) for x in rank_loader.sample()
                ]
                u = model(r_si, r_so, r_oi, r_oo, r_es, r_eo)
                sign = 1.0 - 2.0 * r_par
                upv = torch.sigmoid(u * sign).view(-1, 3)
                hinge = F.relu(args.rank_margin + upv[:, 1:3] - upv[:, 0:1])
                rank_loss = hinge.mean()
                loss = loss + args.rank_alpha * rank_loss
                rank_acc += rank_loss.item()
                rank_fire += (hinge > 0).float().mean().item()
                rank_n += 1
            loss.backward()
            optimizer_dense.step()
            if optimizer_ft is not None:
                optimizer_ft.step()
            scheduler_dense.step()
            if scheduler_ft is not None:
                scheduler_ft.step()
            model.clip_weights()
            if args.ft_clip > 0 and step % args.ft_clip_interval == 0:
                model.clip_ft_weights(args.ft_clip)
            if args.side_clip > 0 and step % args.ft_clip_interval == 0:
                model.clip_eff_weights(args.side_clip)

            step += 1
            samples_done += n
            samples_log += n
            loss_acc += value_loss.item() * n
            loss_n += n

            if step % args.log_interval == 0:
                elapsed = time.time() - t_log
                sps = samples_log / max(elapsed, 1e-6)
                avg_loss = loss_acc / max(loss_n, 1)
                current_lr = scheduler_dense.get_last_lr()[0]
                # ランキング損失と発火率。λを掛ける前の値を出す。
                # 掛けたあとの値ではλを変えるたびに指標が動く
                rank_str = (
                    f" rank {rank_acc / rank_n:.5f}"
                    f" fire {100.0 * rank_fire / rank_n:.1f}%"
                    if rank_n
                    else ""
                )
                print(
                    f"step {step} samples {samples_done} "
                    f"loss {avg_loss:.5f} lr {current_lr:.6f}"
                    f"{rank_str} "
                    f"({sps:.0f} samples/s)",
                    file=sys.stderr,
                )
                if rank_n:
                    if writer:
                        writer.add_scalar("train/rank", rank_acc / rank_n, step)
                        writer.add_scalar("train/rank_fire", rank_fire / rank_n, step)
                    rank_acc = 0.0
                    rank_fire = 0.0
                    rank_n = 0
                if writer:
                    writer.add_scalar("train/loss", avg_loss, step)
                    writer.add_scalar("train/lr", current_lr, step)
                    writer.add_scalar("train/sps", sps, step)
                if log_file:
                    total_elapsed = time.time() - t0
                    log_file.write(
                        f"train\t{step}\t{epoch}\t{samples_done}\t"
                        f"{avg_loss:.5f}\t{current_lr:.6f}\t"
                        f"{sps:.0f}\t{total_elapsed:.1f}\n"
                    )
                    log_file.flush()
                loss_acc = 0.0
                loss_n = 0
                t_log = time.time()
                samples_log = 0

            if valid_loader and step % args.valid_interval == 0:
                # 検証と書き出しは制約を満たした状態で行う
                if args.ft_clip > 0:
                    model.clip_ft_weights(args.ft_clip)
                if args.side_clip > 0:
                    model.clip_eff_weights(args.side_clip)
                vl = validate(model, valid_loader, device)
                print(f"  valid loss {vl:.5f}", file=sys.stderr)
                if writer:
                    writer.add_scalar("valid/loss", vl, step)
                if log_file:
                    total_elapsed = time.time() - t0
                    log_file.write(
                        f"valid\t{step}\t{epoch}\t{samples_done}\t"
                        f"{vl:.5f}\t\t\t{total_elapsed:.1f}\n"
                    )
                    log_file.flush()

                if vl < best_valid:
                    best_valid = vl
                    best_step = step
                    no_improve = 0
                    best_path = f"{args.out}.best"
                    lineage = (
                        f"train-v2-pytorch data={args.data} n={data_n} "
                        f"step={step} valid_loss={vl:.5f} "
                        f"batch={args.batch} peak_lr={args.peak_lr} "
                        f"lambda={args.lambda_}{eff_tag}"
                    )
                    save_hmwr(model, lineage, best_path)
                    print(
                        f"  best checkpoint: {best_path} "
                        f"(step {step}, valid {vl:.5f})",
                        file=sys.stderr,
                    )
                    if args.checkpoint_dir:
                        _save_checkpoint(
                            model, optimizer_dense, optimizer_ft,
                            scheduler_dense, scheduler_ft,
                            step, epoch, best_valid, best_step, samples_done,
                            os.path.join(args.checkpoint_dir, "best.ckpt"),
                        )
                else:
                    no_improve += 1

                if args.checkpoint_dir:
                    _save_checkpoint(
                        model, optimizer_dense, optimizer_ft,
                        scheduler_dense, scheduler_ft,
                        step, epoch, best_valid, best_step, samples_done,
                        os.path.join(args.checkpoint_dir, "latest.ckpt"),
                    )

                if args.patience > 0 and no_improve >= args.patience:
                    print(
                        f"Early stopping: {no_improve}回改善なし "
                        f"(patience={args.patience})",
                        file=sys.stderr,
                    )
                    early_stopped = True
                    break

        if early_stopped:
            break
        print(f"epoch {epoch + 1} 完了", file=sys.stderr)

    elapsed_total = time.time() - t0
    if valid_loader:
        final_valid = validate(model, valid_loader, device)
        print(
            f"最終valid loss {final_valid:.5f} "
            f"(best {best_valid:.5f} at step {best_step})",
            file=sys.stderr,
        )
    else:
        final_valid = float("nan")

    print(
        f"学習完了: {step}ステップ {samples_done}局面 {elapsed_total:.1f}秒",
        file=sys.stderr,
    )

    lineage = (
        f"train-v2-pytorch data={args.data} n={data_n} "
        f"epochs={args.epochs} batch={args.batch} "
        f"peak_lr={args.peak_lr} min_lr={args.min_lr} "
        f"warmup={args.warmup_steps} lambda={args.lambda_} "
        f"steps={step}{eff_tag}"
    )
    if args.side_clip > 0:
        model.clip_eff_weights(args.side_clip)
    save_hmwr(model, lineage, args.out)
    print(f"{args.out} を書き出しました", file=sys.stderr)

    if args.registry:
        _append_registry(args, args.data, data_n, step, total_steps,
                         best_step, best_valid, final_valid, elapsed_total)

    if writer:
        writer.close()
    if log_file:
        log_file.close()


def _save_checkpoint(model, optimizer_dense, optimizer_ft,
                     scheduler_dense, scheduler_ft,
                     step, epoch, best_valid, best_step, samples, path):
    torch.save({
        "model": model.state_dict(),
        "optimizer_dense": optimizer_dense.state_dict(),
        "optimizer_ft": None if optimizer_ft is None else optimizer_ft.state_dict(),
        "scheduler_dense": scheduler_dense.state_dict(),
        "scheduler_ft": None if scheduler_ft is None else scheduler_ft.state_dict(),
        "step": step,
        "epoch": epoch,
        "best_valid": best_valid,
        "best_step": best_step,
        "samples": samples,
    }, path)


def _append_registry(args, data_desc, data_n, step, total_steps,
                     best_step, best_valid, final_valid, elapsed):
    is_new = not os.path.exists(args.registry)
    with open(args.registry, "a", newline="") as f:
        w = csv.writer(f, delimiter="\t")
        if is_new:
            w.writerow([
                "timestamp", "name", "data", "data_n", "epochs", "batch",
                "peak_lr", "min_lr", "warmup", "lambda", "best_step",
                "best_valid", "final_valid", "total_steps", "elapsed_s", "notes",
            ])
        import datetime
        w.writerow([
            datetime.datetime.now().isoformat(timespec="seconds"),
            args.name, data_desc, data_n, args.epochs, args.batch,
            args.peak_lr, args.min_lr, args.warmup_steps, args.lambda_,
            best_step, f"{best_valid:.5f}", f"{final_valid:.5f}",
            total_steps, f"{elapsed:.0f}", args.notes,
        ])


if __name__ == "__main__":
    main()
