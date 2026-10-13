# 第三方许可与来源

## 官方郊狼波形

`crates/controller/data/official-coyote.json` 的名称与波形帧来自 DG-Lab 官方 `dungeonlab-open/dglab-kit` 的 `src/waveform/coyote.ts`，以 GPL-3.0 许可使用，未引入或运行其 SDK 代码。

来源：https://github.com/dungeonlab-open/dglab-kit/blob/c66dea29c10475d5e2ded8bce076a3f2b0d55168/src/waveform/coyote.ts

该数据遵循根目录 LICENSE 中的 GNU GPL v3；数据格式转换不改变其来源与许可。

## GMod 图标

`desktop/src/assets/gmod-icon.png` 与应用 ICO 基于本地 Garry's Mod Windows x64 程序图标，仅调整颜色，保留原字形与透明度。Garry's Mod 标志属于其相应权利人，不作为本项目原创标志或官方认可声明。

## 项目代码与依赖

当前整体分发采用 GPL-3.0-only；此前以 MIT 发布的原创代码仍可按其既有 MIT 授权使用，原许可保存在 LICENSE-MIT。新代码及与官方波形数据组合的当前版本采用 GPL-3.0-only。

Rust 与前端第三方依赖分别遵循自身许可证，版本记录在 Cargo.lock 与 desktop/package-lock.json。
