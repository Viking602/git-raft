# git-raft author — 提交人重写与项目级配置

**日期**: 2026-04-02
**状态**: 已确认

## 目标

新增 `git-raft author` 子命令，解决项目未配置 author 导致提交使用全局默认 author 的问题。支持：

1. 设置项目级 author 配置
2. 自动检测并覆写使用了错误 author 的最近连续提交
3. 分层的 force 控制，安全处理已推送远端的情况

## 命令接口

```bash
# 基础用法：设置项目 author + 修正本地提交
git-raft author --name "Viking" --email "viking@example.com"

# 强制模式：允许改写已推送的提交
git-raft author --name "Viking" --email "viking@example.com" --force

# 强制 + 推送：改写后自动 force push
git-raft author --name "Viking" --email "viking@example.com" --force --push
```

### CLI 参数

| 参数 | 必填 | 说明 |
|------|------|------|
| `--name` | 是 | 项目提交人名称 |
| `--email` | 是 | 项目提交人邮箱 |
| `--force` | 否 | 允许改写已推送到远端的提交 |
| `--push` | 否 | 改写后自动 force push（必须配合 `--force`） |

### 约束

- `--push` 单独使用报错，必须搭配 `--force`
- 不传 `--force` 时，如果检测到待改写的提交已推送远端，报错并提示用户加 `--force`

## 核心逻辑流程

```
git-raft author --name "X" --email "x@example.com"
│
├─ 1. 读取全局 git config 的 user.name / user.email
│
├─ 2. 从 HEAD 连续回扫提交
│     ├─ 提交 author == 全局 config → 标记为待修正
│     └─ 提交 author != 全局 config → 停止扫描
│
├─ 3. 分支判断
│     │
│     ├─ 无待修正提交 → 只写项目级 author 配置，结束
│     │
│     └─ 有待修正提交
│           ├─ 检查这些提交是否已推送远端
│           │     ├─ 已推送 + 无 --force → 报错，提示加 --force
│           │     └─ 未推送 或 有 --force → 继续
│           │
│           ├─ 执行 git rebase，逐个 --amend --author 覆写
│           │
│           ├─ 写入项目级 author 配置
│           │
│           └─ 如果 --force --push → 执行 git push --force-with-lease
```

### 关键实现细节

- **已推送检测**: 对比 `HEAD` 与 `origin/<branch>` 的位置，如果待修正的提交在 `origin` 之前或等于 `origin`，说明已推送
- **覆写方式**: 使用 `git rebase -x 'git commit --amend --author="Name <email>" --no-edit'` 进行批量改写
- **force push**: 使用 `--force-with-lease` 而非 `--force`，更安全
- **项目级配置**: 写入 `.git/config`（`git config --local`），不在 git-raft config 中存储 author，这样 git 原生操作也能受益

## 架构集成

### 新增文件

- `src/commands/author.rs` — 命令入口、`AuthorRun` 结构体、`run_author` 函数

### 修改文件

| 文件 | 改动 |
|------|------|
| `src/cli.rs` | 新增 `CommandKind::Author` 变体 |
| `src/app/dispatch.rs` | 新增 `Author` 分支调度 |
| `src/commands/mod.rs` | 新增 `mod author` + pub use |
| `src/git/worktree.rs` | 新增 `rewrite_author()`、`is_pushed()`、`force_push()` 方法 |
| `src/risk.rs` | `Author` 命令的 risk 分类（有 `--force --push` 时为 High） |

### Hook 集成

- `beforeCommand` / `afterCommand` 正常触发
- 新增 hook 事件 `beforeAuthorRewrite`（在覆写前触发，可被 hook 阻止）
- `--force` 绕过该 hook 的 blocked 状态

## 错误处理

| 场景 | 行为 |
|------|------|
| rebase 过程中冲突 | 中止 rebase，恢复原状，报错提示 |
| `--push` 无 `--force` | 立即报错：`--push requires --force` |
| 远端已推送但无 `--force` | 报错并显示受影响的提交数量和 hash |
| 全局 config 读取失败 | 报错提示检查 git config |
| 没有任何提交（空仓库） | 只写项目级配置，正常结束 |

## 输出设计

```
# 场景 1：无需修正
✓ Project author set: Viking <viking@example.com>
  No commits need rewriting.

# 场景 2：修正成功
✓ Project author set: Viking <viking@example.com>
  Rewrote 3 commits (HEAD~2..HEAD)
  old: Default User <default@example.com>
  new: Viking <viking@example.com>

# 场景 3：需要 force
✗ 3 commits need rewriting, but 2 are already pushed to origin/master.
  Rerun with --force to rewrite, or --force --push to rewrite and push.
```

## 验证方法

- 单元测试：回扫逻辑、已推送检测逻辑
- CLI 集成测试：各场景的端到端行为
- 结构性测试：guardrails 中验证 `CommandKind::Author` 存在且正确调度
