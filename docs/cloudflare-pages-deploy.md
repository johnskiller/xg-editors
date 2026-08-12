# Cloudflare Pages 部署方案

## 1. Cloudflare Pages 与 Private Repo

✅ **支持私有仓库**：Cloudflare Pages 可以连接私有 GitHub 仓库，通过 GitHub App 授权访问。

## 2. 部署方式

### 方式 A: GitHub Integration（推荐）

1. 在 Cloudflare Dashboard 创建 Pages 项目
2. 连接 GitHub 账号，选择私有仓库 `johnskiller/xg-editor`
3. Cloudflare 会自动读取仓库配置并构建部署

**优势**:
- 完全自动化
- 推送即部署
- 与现有 GitHub Actions 配置兼容（可选）

**步骤**:
```bash
# 1. 登录 Cloudflare Dashboard
# https://dash.cloudflare.com

# 2. 进入 Pages → Create a project → Connect to Git
# 3. 选择 GitHub，授权 Cloudflare Pages GitHub App
# 4. 选择仓库: johnskiller/xg-editor
# 5. 配置构建设置:
#    - Framework preset: None
#    - Build command: wasm-pack build --release && wasm-pack publish
#    - Publish directory: www/
# 6. Save and Deploy
```

### 方式 B: CLI 手动部署

```bash
# 安装 wrangler CLI
npm install -g wrangler

# 登录 Cloudflare
wrangler login

# 创建 Pages 项目
wrangler pages project create xg-editor

# 部署
wrangler pages deploy www/ --project-name=xg-editor
```

## 3. 配置对比

| 特性 | GitHub Pages | Cloudflare Pages |
|------|-------------|------------------|
| 私有仓库 | ✅ Pro/Premium | ✅ 免费即可 |
| 自定义域名 | ✅ | ✅ |
| CDN 加速 | ❌ | ✅ |
| 预览分支 | ✅ | ✅ |
| Analytics | ❌ | ✅ |
| Workers 集成 | ❌ | ✅ |

## 4. 现有 GitHub Actions 兼容性

当前 `.github/workflows/deploy.yml` 可以保留，用于：
- 验证构建流程
- 保留 GitHub Pages 作为 fallback

Cloudflare 可以通过以下方式与 GitHub Actions 集成：
- 在 Actions 完成后触发 Cloudflare 部署（Webhook）
- 或者在 Cloudflare 中配置自动同步 GitHub

## 5. 迁移步骤

1. 注册/登录 Cloudflare
2. 创建 Pages 项目，连接 GitHub 私有仓库
3. 配置构建参数（与现有 Actions 相同）
4. 获取自定义域名（可选）
5. 测试部署
6. 更新 DNS 指向 Cloudflare（如需）

## 6. 注意事项

- **GitHub App 权限**: Cloudflare Pages 需要读取仓库内容的权限
- **私有仓库**: 免费版可用，无需付费计划
- **CI/CD**: 可以选择只使用 Cloudflare 的自动构建，或两者都保留

