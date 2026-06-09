# Chat to Chat / 双 AI 多轮执行器

这是一个基于 React、Vite 和 Tauri 2 的桌面应用。它把一次用户目标拆成多轮协作流程：AI1 负责分析、规划和审核，AI2 负责执行。每一轮都会保存 AI1 的方案、AI2 的执行结果、AI1 的审核结论，并根据配置决定是否继续下一轮。

## 功能概览

- 双模型协作：可以分别配置 AI1 和 AI2 的 Base URL、API Key、模型名和温度。
- 多轮执行：设置最大轮数；轮数为 `0` 时表示一直执行到成功或手动停止。
- 成功判断：支持 AI1 判断、关键词匹配、用户手动停止三种模式。
- 任务控制：运行中任务支持暂停、继续、停止和删除。
- 任务历史：任务记录保存在本机应用数据目录，重启后仍可查看。
- 展示模式：支持双栏、时间线和当前轮聚焦三种视图。
- 本地路径授权：可以为 AI2 添加允许读取/写入的文件或目录。
- 文件工具能力：AI2 可在授权路径内读取文件、搜索文件、写入文件，并做基础 PE 文件分析。
- 主题切换：支持深色和浅色主题。

## 技术栈

- 前端：React 18、TypeScript、Vite、lucide-react
- 桌面端：Tauri 2
- 后端：Rust、Tokio、Reqwest、Serde
- 数据存储：本地 JSON 文件

## 目录结构

```text
chat-to-chat/
├─ README.md
├─ .gitignore
└─ publish-single/
   ├─ index.html
   ├─ package.json
   ├─ package-lock.json
   ├─ vite.config.ts
   ├─ tsconfig.json
   ├─ src/
   │  ├─ App.tsx
   │  ├─ main.tsx
   │  └─ styles.css
   └─ src-tauri/
      ├─ Cargo.toml
      ├─ tauri.conf.json
      ├─ capabilities/
      │  └─ default.json
      └─ src/
         ├─ lib.rs
         └─ main.rs
```

## 环境要求

请先安装：

- Node.js 18 或更高版本
- npm
- Rust stable
- Tauri 2 所需的系统依赖

Windows 环境通常还需要：

- Microsoft C++ Build Tools
- WebView2 Runtime

如果 Tauri 环境不完整，可以参考 Tauri 官方文档安装对应系统依赖。

## 安装依赖

进入应用目录：

```powershell
cd publish-single
```

安装前端依赖：

```powershell
npm install
```

Rust 依赖会在第一次运行或构建 Tauri 时自动下载。

## 本地开发

仅启动网页预览：

```powershell
npm run dev
```

网页预览只能查看界面。因为浏览器环境没有 Tauri 后端能力，开始任务、测试模型连接、选择本地路径等功能需要在桌面应用中运行。

启动 Tauri 桌面开发模式：

```powershell
npm run tauri dev
```

开发模式会先启动 Vite，再打开 Tauri 桌面窗口。

## 构建

构建前端：

```powershell
npm run build
```

构建桌面安装包：

```powershell
npm run tauri build
```

构建产物会生成到 `publish-single/src-tauri/target/` 下。该目录体积较大，已经被 `.gitignore` 忽略，不应该提交到 Git。

## 使用流程

1. 打开应用后，点击右上角设置按钮。
2. 分别配置 AI1 和 AI2：
   - `Base URL`：兼容 OpenAI Chat Completions 的接口地址，例如 `https://api.openai.com/v1`。
   - `API Key`：对应服务商的密钥。
   - `Model`：模型名称。
   - `Temperature`：模型随机性。
   - `System Prompt`：当前角色的系统提示词。
3. 点击“测试连接”确认模型配置可用。
4. 按需选择展示模式、成功判断方式和主题。
5. 如果希望 AI2 读取或写入本地文件，在“AI2 本地读写路径”里添加文件或目录。
6. 保存设置。
7. 在主界面输入用户需求，设置轮数，点击“开始”。
8. 运行过程中可以暂停、继续或停止任务。

## AI1 和 AI2 的职责

AI1 负责：

- 理解用户目标。
- 拆解当前轮的执行方案。
- 审核 AI2 的输出。
- 判断目标是否完成。

AI2 负责：

- 根据 AI1 的方案执行任务。
- 在被授权的路径内读取、搜索或写入文件。
- 返回执行结果，供 AI1 审核。

这种设计适合需要“规划 - 执行 - 审核 - 继续修正”的任务，例如代码修改、文档整理、文件分析、批量处理计划等。

## 成功判断模式

`AI1 判定`：

应用会检查 AI1 审核文本中是否包含成功含义，并避免把“需要继续”误判为完成。这是默认模式。

`关键词`：

当 AI2 输出或 AI1 审核中包含配置的关键词时，任务被视为成功。关键词以逗号分隔。

`用户手动`：

应用不会自动判定成功，需要用户手动停止任务。

## 本地文件权限

AI2 只能访问你在设置中选择的文件或目录。

支持的工具能力包括：

- 列出目录内容。
- 分段读取文件。
- 写入文件。
- 查看文件基础信息。
- 搜索文件名。
- 搜索文本内容。
- 基础 PE 文件分析。

写文件有两种方式：

- AI2 使用内部工具写入授权路径。
- AI2 输出如下格式，后端会在授权路径内写入：

```xml
<write_file path="D:\example\output.txt">
文件内容
</write_file>
```

如果目标路径不在授权范围内，后端会拒绝写入。

## 数据存储

应用会把设置和任务历史保存为本地 JSON 文件：

```text
publish-single-data.json
```

该文件位于 Tauri 提供的应用数据目录中，不在项目仓库内。

注意：当前版本的 API Key 会以明文形式保存在本地应用数据文件里。请只在可信设备上使用，不要把本地应用数据文件分享给别人。

## 常用脚本

在 `publish-single` 目录下可用：

```powershell
npm run dev
```

启动 Vite 前端开发服务器。

```powershell
npm run build
```

运行 TypeScript 检查并构建前端静态资源。

```powershell
npm run preview
```

预览已经构建好的前端产物。

```powershell
npm run tauri dev
```

启动 Tauri 桌面开发模式。

```powershell
npm run tauri build
```

构建桌面应用安装包。

## 测试

Rust 后端包含单元测试。运行：

```powershell
cd publish-single\src-tauri
cargo test
```

前端当前没有单独的测试脚本，主要通过 TypeScript 构建检查：

```powershell
cd publish-single
npm run build
```

## Git 提交说明

仓库会提交源码、配置文件、锁文件和文档。

不会提交：

- `node_modules/`
- `dist/`
- `src-tauri/target/`
- `src-tauri/gen/`
- 日志文件
- 本地 `.env` 文件
- 编辑器本地配置

首次推送到远程仓库时，需要先添加 remote，例如：

```powershell
git remote add origin <你的仓库地址>
git push -u origin main
```

后续提交只需要：

```powershell
git add .
git commit -m "你的提交信息"
git push
```

## 常见问题

### 浏览器预览里不能开始任务

这是正常现象。任务执行、模型连接测试、本地路径选择都依赖 Tauri 后端，请使用：

```powershell
npm run tauri dev
```

### 模型连接失败

请检查：

- Base URL 是否正确。
- API Key 是否有效。
- Model 名称是否存在。
- 当前网络是否能访问模型服务。
- 服务商接口是否兼容 OpenAI Chat Completions。

### 构建失败

请检查：

- Node.js、npm、Rust 是否安装。
- Windows C++ Build Tools 是否安装。
- WebView2 Runtime 是否安装。
- 是否在 `publish-single` 目录运行 npm 命令。

### AI2 无法读取或写入文件

请先在设置里添加对应文件或目录。AI2 只能访问授权路径，未授权路径会被拒绝。
