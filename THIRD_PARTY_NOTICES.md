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

## 设备连接新增依赖

设备通信与二维码生成使用下列 Rust 库，均选择其 MIT 授权分发：

- qrcode 0.14.1：https://github.com/kennytm/qrcode-rust
- native-tls 0.2.18（Windows 使用系统 Schannel）：https://github.com/rust-native-tls/rust-native-tls
- url 2.5.8：https://github.com/servo/rust-url

下列许可原文来自随 Cargo 下载的对应版本源码包。

### qrcode-0.14.1

```text
Copyright (c) 2016 kennytm

Permission is hereby granted, free of charge, to any person obtaining a copy of
this software and associated documentation files (the "Software"), to deal in
the Software without restriction, including without limitation the rights to
use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of
the Software, and to permit persons to whom the Software is furnished to do so,
subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS
FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR
COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER
IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN
CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.


```

### native-tls-0.2.18

```text
Copyright (c) 2016 The rust-native-tls Developers

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

```

### url-2.5.8

```text
Copyright (c) 2013-2025 The rust-url developers

Permission is hereby granted, free of charge, to any
person obtaining a copy of this software and associated
documentation files (the "Software"), to deal in the
Software without restriction, including without
limitation the rights to use, copy, modify, merge,
publish, distribute, sublicense, and/or sell copies of
the Software, and to permit persons to whom the Software
is furnished to do so, subject to the following
conditions:

The above copyright notice and this permission notice
shall be included in all copies or substantial portions
of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF
ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED
TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A
PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT
SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY
CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION
OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR
IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
DEALINGS IN THE SOFTWARE.

```