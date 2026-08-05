"""NNUE trainer v2 (ADR-0040, ADR-0045)."""

import argparse
import csv
import math
import multiprocessing
import os
import sys
import time

import torch
import torch.nn as nn
import torch.nn.functional as F
from torch.utils.data import DataLoader
from torch.utils.tensorboard import SummaryWriter

import himawari
from model import EFFECT_LEN, EFFECT_SCALE, FT_IN, NnueModel, effect_loss_fn, loss_fn
from optim import MaskedAdam
from dataset import GeneratedBatchLoader, PsvBatchLoader, PsvDataset, collate_psv
from quantize import save_hmwr


def lr_lambda(step, warmup_steps, total_steps, min_lr, peak_lr):
    if step < warmup_steps:
        return step / max(warmup_steps, 1)
    progress = (step - warmup_steps) / max(total_steps - warmup_steps, 1)
    ratio = min_lr / peak_lr
    return ratio + (1.0 - ratio) * 0.5 * (1.0 + math.cos(math.pi * progress))


def load_teacher(path, device):
    """蒸留の教師からFTだけを読む（ADR-0132）。

    戻り値は (埋め込み, バイアス, FT幅, 元ファイルの構成)。生の次元で読む
    ため `load_hmwr_ft` を使う。`load_hmwr` はいまのビルド構成へ合わせるので、
    FT256の拡張からFT768の教師を読むとFTが切り詰められる。

    教師は更新しない。すべて requires_grad=False にし、optimizerへも渡さない。
    """
    w = himawari.load_hmwr_ft(path)
    ft_out = w["ft_out"]
    ft = nn.EmbeddingBag(FT_IN, ft_out, mode="sum", sparse=False)
    with torch.no_grad():
        ft.weight.copy_(torch.from_numpy(w["ft_w"]).float().view(FT_IN, ft_out))
    ft.weight.requires_grad_(False)
    bias = torch.from_numpy(w["ft_b"]).float().view(ft_out)
    bias.requires_grad_(False)
    return ft.to(device), bias.to(device), ft_out, w["src_arch"]


def teacher_repr(ft, bias, stm_i, stm_o, opp_i, opp_o):
    """教師のFT出力を2視点ぶん連結して返す（ADR-0132）。

    生徒の `transform_both` と同じ計算をする。後段が実際に見るのは
    clamp後の値なので、教師側も同じくclampしてから的にする。
    """
    with torch.no_grad():
        z_stm = (ft(stm_i, stm_o) + bias).clamp(0.0, 1.0)
        z_opp = (ft(opp_i, opp_o) + bias).clamp(0.0, 1.0)
        return torch.cat([z_stm, z_opp], dim=1)


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
            out = model(stm_i, stm_o, opp_i, opp_o)
            loss = loss_fn(out, targets)
            total_loss += loss.item() * targets.size(0)
            total_n += targets.size(0)
    model.train()
    return total_loss / max(total_n, 1)


def main():
    p = argparse.ArgumentParser(description="NNUE trainer v2")
    p.add_argument("--data", help="Training PSV file")
    p.add_argument(
        "--generate",
        type=int,
        help="教師データの代わりに局面をその場で作る（ADR-0133）。値は1エポック"
             "あたりの局面数。初期局面からランダムに指して局面を集め、利きラベルを"
             "その場で計算する。--data の代わりに使い、--lambda-value 0 と組む",
    )
    p.add_argument("--valid", help="Validation PSV file")
    p.add_argument("--out", required=True, help="Output .hmwr path")
    p.add_argument("--epochs", type=int, default=1)
    p.add_argument("--batch", type=int, default=16384)
    p.add_argument("--peak-lr", type=float, default=1e-3)
    p.add_argument("--min-lr", type=float, default=1e-6)
    p.add_argument("--warmup-steps", type=int, default=100)
    p.add_argument("--lambda", type=float, default=0.7, dest="lambda_")
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
        "--lambda-move",
        type=float,
        default=0.0,
        dest="lambda_move",
        help="指し手を当てる補助ヘッドの重み（ADR-0129）。0で無効。"
             "書き出し時にヘッドは捨てるので推論は変わらない",
    )
    p.add_argument(
        "--pretrain",
        action="store_true",
        help="FT事前学習モード（ADR-0129）。評価値ヘッドも線形1層にして、"
             "すべての仕事をFTへ押し込む。成果物はFTだけで、後段は捨てる",
    )
    p.add_argument(
        "--distill-net",
        dest="distill_net",
        help="表現蒸留の教師にする.hmwr（ADR-0132）。指定すると蒸留を有効にする。"
             "太いFTの出力を的にして、細いFTへ表現を写す。読むのはFTだけで、"
             "教師の後段は使わない",
    )
    p.add_argument(
        "--lambda-distill",
        type=float,
        default=0.0,
        dest="lambda_distill",
        help="表現蒸留の重み（ADR-0132）。0.01以下から振る。写像は書き出し時に"
             "捨てるので推論は変わらない",
    )
    p.add_argument(
        "--effect-head",
        dest="effect_head",
        choices=["linear", "mlp"],
        help="利き予測ヘッドを付けてFTを事前学習する（ADR-0133）。linearは線形1層、"
             "mlpは中間256の2層。どちらが良い表現を作るかは比較軸で、決め打たない。"
             "ヘッドは書き出し時に捨てるので推論は変わらない",
    )
    p.add_argument(
        "--lambda-value",
        type=float,
        default=1.0,
        dest="lambda_value",
        help="評価値損失の重み（ADR-0133）。0にすると評価値を使わずに学習する。"
             "利き予測だけでFTを事前学習する第1段階で使う。書き出したネットの"
             "後段は意味を持たないので、--init-net で読み直して第2段階を回す",
    )
    p.add_argument(
        "--lambda-effect",
        type=float,
        default=0.0,
        dest="lambda_effect",
        help="利き予測の重み（ADR-0133）。--effect-head と対で渡す。"
             "λは値ではなく λ×利き損失÷value損失 の割合で決める",
    )
    p.add_argument(
        "--init-net",
        help="既存の.hmwrを初期値に読む。FTは常に読み、後段は形が一致する層だけ"
             "読む（ADR-0130）",
    )
    p.add_argument(
        "--freeze-ft",
        action="store_true",
        help="FTを更新しない。後段の候補を絞るときに使う（ADR-0130）。"
             "採用の判断には使わない",
    )
    p.add_argument(
        "--ft-clip",
        type=float,
        default=0.0,
        help="畳み込み後のFT重みをこの絶対値へ収める（ADR-0138。0=無効）。"
             "i8で格納するとき飽和させないための制約で、量子化スケール127に"
             "対しては1.0を指定する",
    )
    p.add_argument(
        "--ft-clip-interval",
        type=int,
        default=50,
        help="FT重みの射影を何ステップおきに行うか（ADR-0138）。毎ステップは"
             "重すぎる。検証と書き出しの直前はこの設定に関係なく必ず射影する",
    )
    p.add_argument(
        "--seed",
        type=int,
        help="モデル初期化とデータ順序の乱数種。指定しないとPyTorchが実行ごとに"
             "違う種を引くため、同じ条件でもvalid lossが動く（ADR-0127）",
    )
    p.add_argument(
        "--mmap",
        action="store_true",
        help="学習データをmmapで開く（RAMに載らない規模用。速度は落ちる）",
    )
    p.add_argument(
        "--batch-loader",
        action="store_true",
        help="バッチ一括抽出のローダを使う（ADR-0065）",
    )
    p.add_argument(
        "--factorized",
        action="store_true",
        help="学習時のみ駒単独の仮想特徴を併用する（ADR-0066）",
    )
    p.add_argument(
        "--dense-ft",
        action="store_true",
        help="FT勾配をdenseにする（SparseAdamを外し、MPSで学習できる。ADR-0064）",
    )
    args = p.parse_args()

    # 利き予測はヘッドと重みが対で要る（ADR-0133）。片方だけ渡すと、的の
    # ないヘッドを持つか、学習に効かないラベルを抽出し続けることになる
    if args.lambda_effect > 0 and args.effect_head is None:
        p.error("--lambda-effect には --effect-head が要る（ヘッドがない）")
    if args.effect_head is not None and args.lambda_effect <= 0:
        p.error("--effect-head には正の --lambda-effect が要る（重み0では学べない）")
    if args.lambda_value <= 0 and args.effect_head is None:
        p.error("--lambda-value 0 には別の的が要る（--effect-head を渡す）")
    use_effect = args.effect_head is not None

    # 局面の出どころは1つに決める（ADR-0133）。両方渡せるとどちらで学習した
    # のか記録から読めなくなる
    if args.data and args.generate:
        p.error("--data と --generate は同時に使えない（局面の出どころは1つ）")
    if not args.data and not args.generate:
        p.error("--data か --generate のどちらかが要る")
    if args.generate:
        if args.generate <= 0:
            p.error("--generate は正の局面数が要る")
        # 生成した局面に評価値の的はない。targetsは0.5で埋まるので、
        # λ_value>0 のまま回すと定数を当てるだけの学習になる
        if args.lambda_value > 0:
            p.error("--generate には --lambda-value 0 が要る（評価値の的がない）")
        # --valid は要求しない。**生成した局面は使い捨てで、同じ局面が
        # 二度と出ない。** 訓練損失がそのまま未見データの損失になるので、
        # 別の検証集合を持つ意味がない（ADR-0133）
        if args.valid:
            print("--generate では訓練損失が未見データの損失になる。"
                  "--valid は補助的な物差しにしかならない", file=sys.stderr)

    device = torch.device(args.device)
    print(f"Device: {device}", file=sys.stderr)

    # 種を固定しないと初期化とデータ順が実行ごとに変わり、条件の差と
    # 初期値の差を分けられない（ADR-0127）
    if args.seed is not None:
        torch.manual_seed(args.seed)
        print(f"Seed: {args.seed}", file=sys.stderr)

    if args.generate:
        # 抽出ではなく生成。バッチの形は PsvBatchLoader と同じ9本になる
        train_loader = GeneratedBatchLoader(
            args.generate, args.batch, seed=args.seed or 0,
        )
        data_n = train_loader.n
    elif args.batch_loader:
        train_loader = PsvBatchLoader(
            args.data, args.batch, lambda_=args.lambda_,
            score_limit=args.score_limit, mmap=args.mmap, shuffle=True,
            score_clamp=args.score_clamp, seed=args.seed or 0,
            effect=use_effect,
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
    # 系譜と実験台帳へ書く局面の出どころ。生成にはファイル名がない
    data_desc = args.data if args.data else "generated"
    steps_per_epoch = math.ceil(data_n / args.batch)
    total_steps = args.epochs * steps_per_epoch

    valid_loader = None
    if args.valid:
        if args.batch_loader:
            # validには score_limit も score_clamp も適用しない。
            # 教師信号の作り方を変えると物差しが変わり、条件間で
            # valid loss を比べられなくなる（ADR-0126）
            valid_loader = PsvBatchLoader(
                args.valid, args.batch, lambda_=args.lambda_, shuffle=False,
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

    # 教師はモデルより先に読む。写像の出力幅が教師のFT幅で決まる（ADR-0132）
    teacher_ft = None
    teacher_bias = None
    distill_out = 0
    if args.distill_net:
        teacher_ft, teacher_bias, teacher_out, teacher_arch = load_teacher(
            args.distill_net, device,
        )
        # 2視点の連結なので写像の出力は教師のFT幅の2倍になる
        distill_out = teacher_out * 2
        print(
            f"蒸留の教師: {args.distill_net} (構成 {teacher_arch}、"
            f"ft_out={teacher_out}、写像 {distill_out}次元、"
            f"λ={args.lambda_distill})",
            file=sys.stderr,
        )
    elif args.lambda_distill > 0:
        p.error("--lambda-distill には --distill-net が要る（教師がない）")

    if use_effect:
        print(
            f"利き予測: {args.effect_head}ヘッド（λ={args.lambda_effect}）",
            file=sys.stderr,
        )

    model = NnueModel(
        sparse_ft=not args.dense_ft,
        factorized=args.factorized,
        policy=args.lambda_move > 0,
        pretrain=args.pretrain,
        distill_out=distill_out,
        effect_head=args.effect_head,
    ).to(device)

    if args.init_net:
        from quantize import load_into
        print(f"初期値: {load_into(model, args.init_net, args.freeze_ft)}",
              file=sys.stderr)
    elif args.freeze_ft:
        p.error("--freeze-ft には --init-net が要る（凍結する重みがない）")

    dense_params = [
        model.ft_bias,
        model.l2.weight, model.l2.bias,
        model.out.weight, model.out.bias,
    ]
    # 隠れ層は構成によって数が変わる（ADR-0127）
    for layer in (model.l3, model.l4):
        if layer is not None:
            dense_params.extend([layer.weight, layer.bias])
    # 補助ヘッドも学習する。書き出し時に捨てるので推論には出てこない。
    # 蒸留の写像も同じ扱いにする（ADR-0132）
    for head in (model.policy_from, model.policy_to, model.pretrain_value,
                 model.distill):
        if head is not None:
            dense_params.extend([head.weight, head.bias])
    # 利きヘッドはMLPのこともあるので、パラメータをまとめて足す（ADR-0133）
    if model.effect is not None:
        dense_params.extend(model.effect.parameters())
    ft_params = [model.ft.weight]
    if model.ft_p is not None:
        ft_params.append(model.ft_p.weight)
    # 凍結した重みは更新対象から外す。残すとoptimizerが空の勾配を扱う
    dense_params = [t for t in dense_params if t.requires_grad]
    ft_params = [t for t in ft_params if t.requires_grad]
    lr_fn = lambda step: lr_lambda(
        step, args.warmup_steps, total_steps, args.min_lr, args.peak_lr,
    )
    optimizer_dense = torch.optim.Adam(dense_params, lr=args.peak_lr)
    # FT勾配をdenseにするとMPSへ載せられる。更新則はSparseAdamと
    # 同じ「出現した行だけ動かす」を保つ（ADR-0064）
    # FTを凍結すると更新対象が空になる。optimizerは作らない（ADR-0130）
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
        start_epoch = ckpt["epoch"]
        best_valid = ckpt.get("best_valid", float("inf"))
        best_step = ckpt.get("best_step", 0)

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
    distill_acc = 0.0
    distill_n = 0
    eff_short_acc = 0.0
    eff_long_acc = 0.0
    eff_short_base = 0.0
    eff_long_base = 0.0
    effect_n = 0
    samples_log = 0
    samples_done = step * args.batch
    early_stopped = False

    model.train()
    for epoch in range(start_epoch, args.epochs):
        move_hits = 0
        move_total = 0
        for batch in train_loader:
            if batch is None:
                continue

            # 末尾2本は利きラベル（ADR-0133）。抽出させていなければ空で来る
            stm_i, stm_o, opp_i, opp_o, targets, mv_from, mv_to, \
                eff_short, eff_long = [x.to(device) for x in batch]
            n = targets.size(0)

            optimizer_dense.zero_grad()
            if optimizer_ft is not None:
                optimizer_ft.zero_grad()
            x = model.transform_both(stm_i, stm_o, opp_i, opp_o)
            out = model.value(x)
            # ログとvalidに載せるのは評価値の損失だけにする。合計を載せると
            # λを変えるたびに物差しが動き、過去の学習と比べられなくなる
            value_loss = loss_fn(out, targets)
            # 第1段階（利き予測だけでFTを事前学習する）ではλ_value=0にして
            # 評価値を切る。ログには測るだけの値として残す（ADR-0133）
            loss = args.lambda_value * value_loss
            value_loss = value_loss.detach()
            if model.policy_from is not None:
                # ラベルが取れなかった局面は-1で、ignore_indexが落とす
                lf = model.policy_from(x)
                lt = model.policy_to(x)
                move_loss = F.cross_entropy(lf, mv_from, ignore_index=-1) + \
                    F.cross_entropy(lt, mv_to, ignore_index=-1)
                loss = loss + args.lambda_move * move_loss
                valid_mv = mv_to >= 0
                if valid_mv.any():
                    hit = ((lf.argmax(1) == mv_from) & (lt.argmax(1) == mv_to) & valid_mv)
                    move_hits += int(hit.sum())
                    move_total += int(valid_mv.sum())
            if teacher_ft is not None:
                # 教師のFT出力へ、生徒のFT出力からの線形写像を当てる（ADR-0132）
                t_repr = teacher_repr(
                    teacher_ft, teacher_bias, stm_i, stm_o, opp_i, opp_o,
                )
                distill_loss = F.mse_loss(model.distill(x), t_repr)
                loss = loss + args.lambda_distill * distill_loss
                distill_acc += distill_loss.item() * n
                distill_n += n
            if model.effect is not None:
                # 升ごとの利き数を当てる（ADR-0133）。短い利きは加法で解ける
                # ので、健全性チェックとして別々にログへ出す
                short_loss, long_loss = effect_loss_fn(
                    model.effect(x),
                    eff_short.view(n, EFFECT_LEN),
                    eff_long.view(n, EFFECT_LEN),
                )
                # 出力数が同じなので、平均は324次元全体のMSEに等しい
                effect_loss = 0.5 * (short_loss + long_loss)
                loss = loss + args.lambda_effect * effect_loss
                eff_short_acc += short_loss.item() * n
                eff_long_acc += long_loss.item() * n
                # 自明解（全部0と答える）のMSEを一緒に測る。長い利きは
                # 88%の升がゼロなので、基準なしでは損失の大小を読めない。
                # 学習が自明解を下回っているかだけが意味のある判定になる
                with torch.no_grad():
                    scale = EFFECT_SCALE * EFFECT_SCALE
                    eff_short_base += (
                        eff_short.float().pow(2).mean().item() / scale * n
                    )
                    eff_long_base += (
                        eff_long.float().pow(2).mean().item() / scale * n
                    )
                effect_n += n
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
                # 指し手の的中率。表現が厚くなったかを直接は測れないので、
                # 少なくとも「指し手を学べているか」を見る（ADR-0129）
                hit_rate = (
                    f" move {100.0 * move_hits / move_total:.1f}%"
                    if move_total
                    else ""
                )
                move_hits = 0
                move_total = 0
                # 教師のFT出力との二乗誤差。λを掛ける前の値を出す。
                # 掛けたあとの値ではλを変えるたびに物差しが動く（ADR-0132）
                avg_distill = distill_acc / max(distill_n, 1)
                distill_str = f" distill {avg_distill:.5f}" if distill_n else ""
                # 利きは長短を別々に出す（ADR-0133）。短いほうが落ちなければ
                # 実装かλがおかしい。λを掛ける前の値を出すのは蒸留と同じ
                avg_eff_short = eff_short_acc / max(effect_n, 1)
                avg_eff_long = eff_long_acc / max(effect_n, 1)
                base_s = eff_short_base / max(effect_n, 1)
                base_l = eff_long_base / max(effect_n, 1)
                effect_str = (
                    f" effect short {avg_eff_short:.5f}/{base_s:.5f}"
                    f" long {avg_eff_long:.5f}/{base_l:.5f}"
                    if effect_n
                    else ""
                )
                print(
                    f"step {step} samples {samples_done} "
                    f"loss {avg_loss:.5f} lr {current_lr:.6f}"
                    f"{hit_rate}{distill_str}{effect_str} "
                    f"({sps:.0f} samples/s)",
                    file=sys.stderr,
                )
                if writer:
                    writer.add_scalar("train/loss", avg_loss, step)
                    writer.add_scalar("train/lr", current_lr, step)
                    writer.add_scalar("train/sps", sps, step)
                    if distill_n:
                        writer.add_scalar("train/distill", avg_distill, step)
                    if effect_n:
                        writer.add_scalar("train/effect_short", avg_eff_short, step)
                        writer.add_scalar("train/effect_long", avg_eff_long, step)
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
                distill_acc = 0.0
                distill_n = 0
                eff_short_acc = 0.0
                eff_long_acc = 0.0
                eff_short_base = 0.0
                eff_long_base = 0.0
                effect_n = 0
                t_log = time.time()
                samples_log = 0

            if valid_loader and step % args.valid_interval == 0:
                # 検証と書き出しは制約を満たした状態で行う（ADR-0138）
                if args.ft_clip > 0:
                    model.clip_ft_weights(args.ft_clip)
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
                        f"train-v2-pytorch data={data_desc} n={data_n} "
                        f"step={step} valid_loss={vl:.5f} "
                        f"batch={args.batch} peak_lr={args.peak_lr} "
                        f"lambda={args.lambda_}"
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
        f"train-v2-pytorch data={data_desc} n={data_n} "
        f"epochs={args.epochs} batch={args.batch} "
        f"peak_lr={args.peak_lr} min_lr={args.min_lr} "
        f"warmup={args.warmup_steps} lambda={args.lambda_} "
        f"steps={step}"
    )
    save_hmwr(model, lineage, args.out)
    print(f"{args.out} を書き出しました", file=sys.stderr)

    if args.registry:
        _append_registry(args, data_desc, data_n, step, total_steps,
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
