# NetSpeed Analyzer

ローカルネットワーク速度測定ツール（Windows GUI / 環境構築不要）

## exeの入手方法

### 方法 A：GitHub Actions で自動ビルド（推奨）

1. GitHubに新規リポジトリを作成（Public/Private どちらでも可）

2. このzipの中身をpush
   ```powershell
   git init
   git add .
   git commit -m "init"
   git remote add origin https://github.com/<ユーザー名>/<リポジトリ名>.git
   git push -u origin main
   ```

3. GitHub の **Actions タブ** を開く  
   → `Build Windows EXE` が自動実行される（約5〜10分）

4. 完了後 → **Artifacts** から `netspeed-windows.zip` をダウンロード  
   中に `netspeed.exe` が入っています

### 方法 B：タグを打って Releases に公開

```powershell
git tag v0.1.0
git push origin v0.1.0
```
→ GitHub Releases に `netspeed.exe` が自動アップロードされます

---

## 機能

| 機能 | 詳細 |
|------|------|
| ↓ ダウンロード速度 | Cloudflare CDN から 10MB×3回計測、平均 Mbps |
| ↑ アップロード速度 | 2MB×2回計測、平均 Mbps |
| ◎ Ping / レイテンシ | 5回計測、平均・最小・最大・ジッター |
| 🔍 LAN ホストスキャン | TCP接続で .1〜.254 を並列スキャン |
| 📈 リアルタイムグラフ | 測定値の履歴グラフ |

## 注意

- インターネット接続が必要（speed.cloudflare.com）
- LAN スキャンは TCP ベース（ICMP ブロック環境でも動作）
- Windows Defender 警告が出たら「詳細情報」→「実行」

## ライセンス
MIT
