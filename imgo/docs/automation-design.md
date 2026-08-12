# imgo 自动化工作流设计

## 目标

把你现在的"下载 → 肉眼逐张判参数 → 跑 N 次 imgo → 监控 → 打包 → 搬运"
压缩成"分类 → 改 manifest → 一条命令去睡觉"。

核心思路：**分类和执行解耦**。
classifier 负责把"经验"变成可审的文本 manifest，你扫文本而不是看图；
route 负责按 manifest 把混合图包一次跑完。
两者各自独立，互不依赖，可以分阶段交付。

---

## 现状诊断：为什么现在这么累

根因在代码结构，不在你懒。

- `run_pipeline_external(shared, transcoder)` 一次只吃一个 transcoder（`pipeline.rs:425`）。
  CLI 每个 subcommand（`avif`/`jxl`/`denoise`/`cleanscan`）就是单 transcoder 一次跑（`bin/i.rs`）。
- `SharedOpts`（`pipeline.rs:223`）没有 per-image 参数、没有链式、没有分类。
- 所以混合图包 = 你手动把图拆成 N 个同类子集，跑 N 次命令，监控 N 次。
  第 3 步的劳动本质是这个限制逼出来的。

还有一个痛点：**今天没法链式**。
想 `denoise -> avif`，只能手动跑两遍——`imgo d` 出一批 PNG（备份一次原件），
再 `imgo a` 压这批 PNG（又备份一次）。`.backup` 嵌套、两轮监控、两份备份。
`cleanscan -> jxl` 同理。

---

## 三个新概念

全部叠加在现有 `External`/`Pixel` trait 之上，不推翻现有设计。

### 1. Pipeline = 有序的 transcoder 步骤链

```text
[{denoise, mode: artifact, strength: strong}, {avif, cq: 22}]
```

orchestrator 已经在管 temp/backup/并行（`pipeline.rs:345` 的 `orchestrate`）。
链式只需把"单步 exec"换成"多步串行 exec"：
中间产物走 temp 文件，只有最终输出落盘 + 备份一次。
`process_one`（`pipeline.rs:154`）现在是 `temp → work → backup → dest`，
改成一串 temp 在步骤间传递，最后一步才 backup + 落盘。

步骤间的格式兼容性由 `Meta::input_formats()` / `output_format()` 现成保证：
`denoise` 出 PNG（`magick.rs:58`），`avif` 收 PNG（`avif.rs:69`），链得上。
编排时校验相邻步骤的输出/输入格式匹配，不匹配直接报错，不跑。

### 2. Recipe = 命名的 pipeline + 目标格式

把你的"经验参数"固化成具名配方。内置几个覆盖常见场景：

| Recipe              | 步骤链                                   | 适用                     |
|---------------------|------------------------------------------|--------------------------|
| `clean-color`       | `[avif cq:22]`                           | 干净彩色图               |
| `jpeg-artifact-lg`  | `[denoise artifact strong -> avif cq:22]`| 大图有 JPEG artifact     |
| `jpeg-artifact-sm`  | `[denoise artifact light -> avif cq:22]` | 小图，轻 despeckle 防糊  |
| `screentone`        | `[cleanscan -> jxl]`                      | 网点黑白图，2bit + 无损  |
| `fake-pencil`       | `[denoise fakepencil -> avif cq:22]`      | 假铅笔噪点               |

`avif:cq` / `denoise:strength` 这些现有参数原样复用，不用改 transcoder。
新场景你加个 recipe 条目就行，不动核心。

Recipe 用 TOML/JSON 描述，可内置也可外置文件（`~/.config/imgo/recipes.toml`）。
内置提供默认集，外置让你覆盖或加自己的。

### 3. Manifest = 每张图 → recipe 的映射

```toml
# 自动生成、人工可改
[defaults]
recipe = "clean-color"      # 没单列的图走默认

[[entries]]
path = "001.png"
recipe = "screentone"

[[entries]]
path = "cover.png"
recipe = "clean-color"

[[entries]]
path = "p005.png"
recipe = "jpeg-artifact-lg"
```

可手写，也可由 `imgo classify` 生成草稿。

---

## 命令

```sh
# 扫图、出 manifest 草稿（带置信度 + 理由）
imgo classify <dir> -o manifest.toml

# 按 manifest 跑：每图走自己的 recipe，rayon 并行，
# 一次备份、一个进度条、一次监控
imgo route manifest.toml
```

`route` 是"跑个命令去睡觉"那一步。
mixed 图包从"跑 N 次、监控 N 次"变成"跑 1 次、监控 1 次"。

---

## 分类器：把第 3 步从肉眼降到文本

### 能不能靠谱

能。不需要 ML、不需要新重依赖。
`image` crate 已经在 workspace deps（`Cargo.toml:28`），能 decode PNG/JPEG。
cheap 像素统计覆盖你判断依据里的大部分：

| 你判断的       | 自动化的方法                                              | 成本     |
|----------------|----------------------------------------------------------|----------|
| 彩色 vs 灰度   | 扫像素看 `max(|R-G|,|G-B|,|R-B|)`，超阈值即彩色           | O(n)     |
| 2-tone / 网点  | 直方图强双峰，或唯一颜色数极少                            | O(n)     |
| 尺寸 → 力度    | 直接编码经验：`longest_edge > 2000 → strong` 等           | 读 header|
| JPEG artifact  | 源是 JPG + 8px 块边界方差（blockiness）                  | O(n)     |

前两项 trivial，第三项是把你脑子里的规则写成 config。
最弱是 JPEG artifact 检测，cheap proxy 是 blockiness。
判不准时倾向"按尺寸档 despeckle"——对干净 JPEG 轻微 despeckle 只是略软，
可接受；真不准的你在 manifest 里改。

### 关键：classifier 不需要 100% 对

它出草稿、你改文本、然后执行。
你从"逐张肉眼看图选参数"降到"扫一眼文本 manifest 改几行"。
这才是省注意力的地方。

### Manifest 输出：按摘要审，不是逐行审

200 页的书 manifest 逐行审不现实。
classifier 输出按 recipe 分组的摘要，只把 low-confidence 条目单列出来让人审：

```text
分类摘要：
  clean-color      : 8 张   (封面、彩页)
  screentone       : 187 张 (正文黑白网点)
  jpeg-artifact-lg : 5 张   (来源 JPEG，大图)

需人工确认（置信度 < 0.7）：
  p042.png  → jpeg-artifact-lg? (blockiness 0.62，可能是干净 JPG)
  p150.png  → screentone? (颜色数偏多，可能是灰阶而非纯网点)

其余按上述分组，默认 recipe = clean-color。
```

人从"看 200 行"降到"看几行"。

---

## 自动化清单（按优先级）

每项独立，按需开。

### C1. Manifest + 链式 pipeline + `imgo route` —— 地基

mixed 图包立刻一条命令跑完。
没有 classifier 也能手写 manifest 凑合用。
这是把第 3/4 步的 N 次循环砍成 1 次的关键。

### C2. `imgo classify` —— 第 3 步自动化

第 3 步从"肉眼逐张"降到"扫文本摘要改几行"。
和 C1 解耦，C1 先落地、C2 后补，互不阻塞。

### C3. 多本子批量 + 归档 hook

```sh
imgo batch b1/ b2/ b3/   # 逐个 classify + route + archive
```

真·睡前一键。transcode 完直接调你那个 7z rust app，少一步手动打包。

### C4. 完成通知 + 备份策略

跑完 `notify-send` / 终端 bell，不用切窗看进度。

备份删除要谨慎。你第 5 步是"看一遍结果再删"——
200 张里一张 cleanscan 翻车很常见，
"输出非空"校验替代不了肉眼 QA。
所以默认**不自动删** `.backup`，做成显式 flag：

```sh
imgo route manifest.toml --purge-backup   # 显式确认才删
```

或更稳的：归档成功（7z 打完、校验通过）后才删备份。
默认行为偏向安全，宁可多留一次备份让你手动清。

### C5. 输出落盘到 NAS 挂载点

`imgo route --dest /mnt/nas/manga/...`：路径映射你来定，
连最后搬运都省。

---

## 交付节奏

两个阶段，风险隔离：

1. **地基阶段（C1）**：manifest + 链式 pipeline + `route`。
   classifier 之后再加。立即能用、风险最低。
2. **自动化阶段（C2–C5）**：classifier、批量、归档 hook、通知、NAS 落盘。
   在地基上叠加，每项独立可测。

执行自动化是地基——哪怕没 auto-classifier，
光"manifest + 链式 + 一条命令跑 mixed 包"就把第 3/4 步的 N 次循环砍成 1 次。
classifier 是把第 3 步从"肉眼"降到"扫文本"。
两者解耦，可分两步交付。

---

## 实现对接点（给写代码时的锚）

- `orchestrate`（`pipeline.rs:345`）：核心要改的地方。
  现在 `execute: Fn(&Image, &Path)` 是单步，扩展成步骤链。
  temp 在步骤间传递，最后一步才 backup + 落盘。
- `process_one`（`pipeline.rs:154`）：现在 `temp → work → backup → dest`。
  链式改成一串 temp 传递。
- `Meta::input_formats()` / `output_format()`（`transcoder/mod.rs:22,25`）：
  现成，用来校验相邻步骤格式匹配。
- 现有 transcoder（`avif.rs`/`jxl.rs`/`magick.rs`）的参数：
  recipe 原样复用，不动 transcoder 本身。
- `collect_for`（`pipeline.rs:264`）：manifest 模式下，
  收集逻辑改成"读 manifest 的 entries"而不是按格式扫全目录。
- 新增 `classify` 模块：独立于 pipeline，只做像素分析 + manifest 生成。
  依赖 `image` crate（已在 workspace deps），不引入新重依赖。

---

## 你得到的工作流

```text
下载压缩包 → 解压
  → imgo classify <dir> -o manifest.toml
  → 扫一眼 manifest 摘要，改几行 low-confidence
  → imgo route manifest.toml --archive --notify
  → 去睡觉
醒来 → 检查归档 → 放 NAS
```

第 3 步从"逐张肉眼看图选参数"变成"扫文本摘要改几行"。
第 4 步从"跑 N 次、切窗监控 N 次"变成"一条命令跑一次"。
mental context switching 从"反复切窗看番看进度"降到"睡前一条命令"。
