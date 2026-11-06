# Cloud Run Jobs + Cloud Scheduler への移行・運用手順書

GitHub Actions の cron で動かしていた定期実行を、Cloud Run Jobs と Cloud Scheduler に移すための手順書です。
**このリポジトリのコードや workflow は GCP / GitHub の設定を変更しません。** 以下のコマンドはすべて人間が実行します。

## 全体像

```
main へ push ──> deploy.yml ──> イメージをビルド・push ──> Cloud Run Job を更新（実行はしない）──> 古いイメージを全削除
Cloud Scheduler（08:00 / 18:00 JST）──> Cloud Run Job `github-trend-digest` を実行 ──> Discord に通知
```

- Job は 1 つ（引数なし。production 設定の `target_language = ""` により全言語を実行する）。
- 認証は Workload Identity Federation（SA キーは使わない）。
- 保存するイメージは、デプロイしたものの 1 件だけ。

## 1. 前提と変数

以降のコマンドは、次の変数を設定したシェルで実行します。値は自分の環境に置き換えてください。

```bash
export PROJECT_ID="<your-project-id>"
export REGION="asia-northeast1"          # 推奨
export GAR_REPOSITORY="github-trend-digest"   # Artifact Registry のリポジトリ名
export GH_REPO="tminasen/github-trend-digest"
export IMG="${REGION}-docker.pkg.dev/${PROJECT_ID}/${GAR_REPOSITORY}/github-trend-digest"

export DEPLOYER_SA="gtd-deployer@${PROJECT_ID}.iam.gserviceaccount.com"   # GitHub Actions が使う
export RUNTIME_SA="gtd-runtime@${PROJECT_ID}.iam.gserviceaccount.com"     # Job の実行に使う
export SCHEDULER_SA="gtd-scheduler@${PROJECT_ID}.iam.gserviceaccount.com" # Scheduler が Job を起動するのに使う

gcloud config set project "$PROJECT_ID"
```

固定の名前（workflow に直接書かれている）:

| 種類 | 名前 |
|---|---|
| Cloud Run Job / イメージ名 | `github-trend-digest` |
| Secret | `github-trend-digest-discord-url` / `github-trend-digest-gemini-api-key` / `github-trend-digest-github-token` |

GCP の識別子（プロジェクト ID・リージョンなど）は workflow に書かず、GitHub の Repository variables から読みます（8 章）。

## 2. API の有効化

```bash
gcloud services enable \
  run.googleapis.com artifactregistry.googleapis.com secretmanager.googleapis.com \
  cloudscheduler.googleapis.com iamcredentials.googleapis.com sts.googleapis.com
```

## 3. Artifact Registry のリポジトリ作成

```bash
gcloud artifacts repositories create "$GAR_REPOSITORY" \
  --repository-format=docker --location="$REGION" \
  --description="github-trend-digest images"
```

> Artifact Registry の cleanup policy は使いません。非同期に実行され、デプロイの成否と連動しないためです。
> 古いイメージの削除は deploy workflow の最後のステップ（`scripts/cleanup-old-images.sh`）が行います。

## 4. サービスアカウントと IAM

```bash
gcloud iam service-accounts create gtd-deployer  --display-name="github-trend-digest deployer"
gcloud iam service-accounts create gtd-runtime   --display-name="github-trend-digest runtime"
gcloud iam service-accounts create gtd-scheduler --display-name="github-trend-digest scheduler"

# deployer: Job の作成・更新
gcloud projects add-iam-policy-binding "$PROJECT_ID" \
  --member="serviceAccount:${DEPLOYER_SA}" --role="roles/run.developer"
# deployer: runtime SA を Job に設定する権限
gcloud iam service-accounts add-iam-policy-binding "$RUNTIME_SA" \
  --member="serviceAccount:${DEPLOYER_SA}" --role="roles/iam.serviceAccountUser"
# deployer: イメージの push と削除（リポジトリ単位。writer には delete がないので repoAdmin）
gcloud artifacts repositories add-iam-policy-binding "$GAR_REPOSITORY" --location="$REGION" \
  --member="serviceAccount:${DEPLOYER_SA}" --role="roles/artifactregistry.repoAdmin"

# runtime: 各 secret の読み取り（7 章で secret を作った後に実行する）
for s in discord-url gemini-api-key github-token; do
  gcloud secrets add-iam-policy-binding "github-trend-digest-${s}" \
    --member="serviceAccount:${RUNTIME_SA}" --role="roles/secretmanager.secretAccessor"
done

# scheduler: Job の実行（10 章で Job を作った後に実行する）
gcloud run jobs add-iam-policy-binding github-trend-digest --region="$REGION" \
  --member="serviceAccount:${SCHEDULER_SA}" --role="roles/run.invoker"
```

根拠: https://cloud.google.com/artifact-registry/docs/access-control#roles ・ https://cloud.google.com/run/docs/execute/jobs-on-schedule

## 5. 削除権限の確認

deployer がイメージを削除できること、権限が必要以上に広がっていないことを確かめます。

```bash
# (1) repoAdmin に versions.delete と tags.delete の両方が含まれること
gcloud iam roles describe roles/artifactregistry.repoAdmin \
  --format='value(includedPermissions)' | tr ';' '\n' | grep -E '^artifactregistry\.(versions|tags)\.delete$'
# 期待値: 2 行（artifactregistry.versions.delete / artifactregistry.tags.delete）

# (2) リポジトリ単位で付いていること
gcloud artifacts repositories get-iam-policy "$GAR_REPOSITORY" --location="$REGION"

# (3) プロジェクト単位で余分な権限が付いていないこと（期待値: roles/run.developer だけ）
gcloud projects get-iam-policy "$PROJECT_ID" \
  --flatten=bindings --filter="bindings.members:serviceAccount:${DEPLOYER_SA}" \
  --format='value(bindings.role)'
```

## 6. Workload Identity Federation

GitHub Actions からの認証を、**このリポジトリの main ブランチだけ**に限定します。
リポジトリ名ではなく、数値の `repository_id` / `repository_owner_id` で限定します（名前の付け替えによるなりすましを防ぐため）。

```bash
# ID の調べ方
gh api "repos/${GH_REPO}" --jq '.full_name, .id, .owner.id'
# 1 行目が ${GH_REPO} と一致することを確かめる（別のリポジトリの ID を使わないため）
export REPO_ID="<2 行目>"
export OWNER_ID="<3 行目>"

gcloud iam workload-identity-pools create github --location=global --display-name="GitHub Actions"

gcloud iam workload-identity-pools providers create-oidc github-trend-digest \
  --location=global --workload-identity-pool=github \
  --issuer-uri="https://token.actions.githubusercontent.com" \
  --attribute-mapping="google.subject=assertion.sub,attribute.repository_id=assertion.repository_id,attribute.repository_owner_id=assertion.repository_owner_id,attribute.ref=assertion.ref" \
  --attribute-condition="assertion.repository_id == '${REPO_ID}' && assertion.repository_owner_id == '${OWNER_ID}' && assertion.ref == 'refs/heads/main'"

export PROJECT_NUMBER="$(gcloud projects describe "$PROJECT_ID" --format='value(projectNumber)')"

# deployer SA へのバインド（この repository_id のトークンだけ）
gcloud iam service-accounts add-iam-policy-binding "$DEPLOYER_SA" \
  --role="roles/iam.workloadIdentityUser" \
  --member="principalSet://iam.googleapis.com/projects/${PROJECT_NUMBER}/locations/global/workloadIdentityPools/github/attribute.repository_id/${REPO_ID}"

# workflow の WIF_PROVIDER に設定する値
gcloud iam workload-identity-pools providers describe github-trend-digest \
  --location=global --workload-identity-pool=github --format='value(name)'
```

根拠: https://github.com/google-github-actions/auth#security-considerations

## 7. Secret Manager への登録と GITHUB_TOKEN の運用

値は標準入力から渡し、シェルの履歴に残しません。

```bash
for s in discord-url gemini-api-key github-token; do
  gcloud secrets create "github-trend-digest-${s}" --replication-policy=automatic
done

# 例: 値を入力して Ctrl-D（改行を含めないこと）
printf %s "$DISCORD_WEBHOOK_URL" | gcloud secrets versions add github-trend-digest-discord-url --data-file=-
printf %s "$GEMINI_API_KEY"      | gcloud secrets versions add github-trend-digest-gemini-api-key --data-file=-
printf %s "$GITHUB_PAT"          | gcloud secrets versions add github-trend-digest-github-token --data-file=-
```

(`secretAccessor` の付与は 4 章の続きとしてここで行います。)

### GITHUB_TOKEN（fine-grained PAT）の運用

Actions の組み込み `GITHUB_TOKEN` は Cloud Run では使えないため、PAT を発行して登録します。

| 項目 | 内容 |
|---|---|
| 発行する人 | リポジトリ管理者（人間）のアカウント |
| 権限 | Public repositories（read-only）だけ。Repository permissions は付けない |
| 有効期限 | 1 年（発行日をカレンダーに登録しておく） |
| 更新手順 | `printf %s "$NEW_PAT" \| gcloud secrets versions add github-trend-digest-github-token --data-file=-` |
| 反映 | Job は `:latest` を参照しているので、次回の実行から新しい PAT が使われる（再デプロイ不要） |
| 期限切れの症状 | 起動時の `verify_credentials` が失敗し、run がエラーで終わる。**アプリ内の失敗通知（`GitHub API error: 401 ...`）として Discord に届く** |

## 8. GitHub Repository variables の登録

機密ではないので secrets ではなく variables を使います。

```bash
gh variable set GCP_PROJECT_ID          --repo "$GH_REPO" --body "$PROJECT_ID"
gh variable set GCP_REGION              --repo "$GH_REPO" --body "$REGION"
gh variable set GAR_REPOSITORY          --repo "$GH_REPO" --body "$GAR_REPOSITORY"
gh variable set WIF_PROVIDER            --repo "$GH_REPO" --body "<6 章の最後に出力された provider のリソース名>"
gh variable set DEPLOY_SERVICE_ACCOUNT  --repo "$GH_REPO" --body "$DEPLOYER_SA"
gh variable set RUNTIME_SERVICE_ACCOUNT --repo "$GH_REPO" --body "$RUNTIME_SA"
```

## 9. 既存の ruleset に必須ステータスチェックを追加する

CI の成功は、deploy workflow では待ちません（deploy.yml は独立して動く）。マージ前の CI 成功は、ruleset で担保します。

現状（2026-11-03 時点）の ruleset `protect-main`（id 25413922）には `deletion` / `non_fast_forward` / `pull_request` しかなく、`required_status_checks` がありません。**この章を済ませるまで、CI の成功は担保されません。**

**順序:** 必須にする 2 つのチェック（`Build, Test, and Lint` と `Scripts and Workflows`）が **一度は PR で実行された後** に設定します。新しい job の名前が GitHub にまだ登録されていないと、PR がブロックされたままになるためです。

```bash
# 1. 今の設定を取得する（バックアップを兼ねる）
gh api repos/tminasen/github-trend-digest/rulesets/25413922 > ruleset.json

# 2. 既存の rules を残し、required_status_checks を 1 つだけ入れた body を作る
#    （同じ type の既存ルールは取り除いてから足すので、2 回実行しても重複しない）
jq '{
  name, target, enforcement, conditions, bypass_actors,
  rules: ([.rules[] | select(.type != "required_status_checks")] + [{
    type: "required_status_checks",
    parameters: {
      strict_required_status_checks_policy: false,
      required_status_checks: [
        {context: "Build, Test, and Lint", integration_id: 15368},
        {context: "Scripts and Workflows", integration_id: 15368}
      ]
    }
  }])
}' ruleset.json > ruleset-new.json

# 3. 反映する
gh api -X PUT repos/tminasen/github-trend-digest/rulesets/25413922 --input ruleset-new.json

# 4. 読み直して確かめる
#    期待値: ["deletion","non_fast_forward","pull_request","required_status_checks"]（順不同）と
#           ["Build, Test, and Lint","Scripts and Workflows"]
gh api repos/tminasen/github-trend-digest/rulesets/25413922 \
  --jq '[.rules[].type], [.rules[] | select(.type=="required_status_checks") | .parameters.required_status_checks[].context]'
```

- `15368` は GitHub Actions のアプリ ID です。
- 戻すときは `ruleset.json` をそのまま PUT せず（GET の応答には読み取り専用の項目が含まれるため）、手順 2 の jq で `required_status_checks` を除いた body を作って PUT します。
- 根拠: https://docs.github.com/rest/repos/rules

## 10. 初回のイメージ push と Job 作成

並行稼働での確認（12 章）をマージ前に行うため、最初のイメージ push と Job の作成は手動で行います。マージ後は deploy workflow が Job を上書き更新します。

作業マシンが arm64 の場合、Docker Desktop の buildx は既定で attestation を付けます。CI と同じ「単一 manifest」にするため、**次のコマンドをそのまま使います**（`--provenance=false --sbom=false` を省かない）。

```bash
gcloud auth configure-docker "${REGION}-docker.pkg.dev"

export TAG="manual-$(date +%Y%m%d)"
docker buildx build --platform linux/amd64 --provenance=false --sbom=false \
  -t "${IMG}:${TAG}" --push .

# push した digest を確認する
export DIGEST="$(gcloud artifacts docker images describe "${IMG}:${TAG}" --format='value(image_summary.digest)')"
echo "$DIGEST"

# Job を作成する（フラグは deploy workflow と同じ。secret は 3 つとも :latest）
gcloud run jobs create github-trend-digest --region="$REGION" \
  --image="${IMG}@${DIGEST}" \
  --service-account="$RUNTIME_SA" --max-retries=0 --task-timeout=30m \
  --set-secrets="DISCORD_WEBHOOK_URL=github-trend-digest-discord-url:latest,GEMINI_API_KEY=github-trend-digest-gemini-api-key:latest,GITHUB_TOKEN=github-trend-digest-github-token:latest"

# 実行して確認する（Discord にまとめが届く）
gcloud run jobs execute github-trend-digest --region="$REGION" --wait
```

- `--max-retries=0`: リトライで Discord に二重通知されるのを防ぎます。
- 4 章の最後の `run.invoker` の付与は、ここで Job を作った後に行います。

根拠: https://cloud.google.com/run/docs/create-jobs ・ https://cloud.google.com/run/docs/configuring/jobs/secrets

## 11. Cloud Scheduler の作成

08:00 / 18:00 JST の 2 件を作ります。

```bash
export JOB_URI="https://run.googleapis.com/v2/projects/${PROJECT_ID}/locations/${REGION}/jobs/github-trend-digest:run"

gcloud scheduler jobs create http github-trend-digest-0800 --location="$REGION" \
  --schedule="0 8 * * *" --time-zone="Asia/Tokyo" \
  --uri="$JOB_URI" --http-method=POST \
  --oauth-service-account-email="$SCHEDULER_SA"

gcloud scheduler jobs create http github-trend-digest-1800 --location="$REGION" \
  --schedule="0 18 * * *" --time-zone="Asia/Tokyo" \
  --uri="$JOB_URI" --http-method=POST \
  --oauth-service-account-email="$SCHEDULER_SA"
```

根拠: https://cloud.google.com/run/docs/execute/jobs-on-schedule

## 12. 並行稼働で確認

**短期間だけ**、GitHub Actions の cron と Cloud Scheduler の両方を動かして、出力を比べます。**この間は Discord に同じ内容が二重に届きます。**

1. 11 章の Scheduler を作成する（cron.yml はまだ残っている）。
2. `gcloud scheduler jobs run github-trend-digest-0800 --location="$REGION"` で手動起動し、Discord に届くことを確かめる。
3. 1〜2 回分の定期実行を見て、まとめの内容と件数が cron の結果と同等であることを確かめる。
4. 問題がなければ 13 章に進む。

## 13. マージ

マージには次の変更が含まれます。

- `.github/workflows/cron.yml` の削除（GitHub Actions の定期実行と手動実行がなくなる）
- `.github/workflows/deploy.yml` の追加（main への push ごとに、ビルド → Job 更新 → 古いイメージの全削除）

マージ前に確認すること:

- 8 章の variables が登録済みであること。
- 9 章の ruleset が設定済みであること。
- 12 章の確認が済んでいること。

**手動の初回イメージ（`manual-<YYYYMMDD>` タグ）は、マージ後の最初のデプロイで削除されます。** これで問題ありません。

- cleanup が動くのは deploy-cloudrun の成功後で、その時点で Job はもう新しい digest を参照しています。
- 実行中の execution があっても、コンテナはすでに起動しているので影響しません（Cloud Run は起動時に pull します）。
- 手動イメージが CI のビルドと同じ digest になった場合は、それがデプロイした digest なので残ります。

## 14. マージ後の確認

1. deploy workflow が成功していること。
2. deploy workflow の cleanup ステップのログで、`docker buildx imagetools inspect` が成功していること（失敗していると、何も削除されずに異常終了します）。
3. Artifact Registry の version が、デプロイした digest の **1 件だけ**であること。`manual-` タグがないこと。

   ```bash
   gcloud artifacts docker images list "$IMG" --format='value(version)'
   gcloud run jobs describe github-trend-digest --region="$REGION" \
     --format='value(spec.template.spec.template.spec.containers[0].image)'
   # 上の 1 行と、下の image の @sha256:... が一致すること
   gcloud artifacts docker images list "$IMG" --include-tags --format='value(tags)'   # manual- がないこと
   ```

4. 使わなくなった GitHub secrets を削除する（`GITHUB_TOKEN` は Actions が自動発行するもので、削除の対象外）。

   ```bash
   gh secret delete DISCORD_WEBHOOK_URL --repo "$GH_REPO"
   gh secret delete GEMINI_API_KEY      --repo "$GH_REPO"
   ```

5. 次の定期実行（Scheduler）で、まとめが 1 回だけ届くこと。

### cleanup が失敗したとき

- **`is an image index` / `could not be verified` で止まった:** デプロイしたイメージが単一 manifest でないか、確認できなかったため、何も削除せずに止まっています。`docker/build-push-action` の `provenance: false` / `sbom: false` / `platforms: linux/amd64` が外れていないかを確認してください。
- **index の子 manifest の削除が Artifact Registry に拒否された:** 過去に、この手順書のビルドコマンドを使わずに push したイメージ（image index）が残っている場合に起きえます。Artifact Registry のコンソールで、その index（親）を先に削除してから、workflow を再実行してください。デプロイしたイメージには影響しません。
- **削除の途中で失敗した（権限・一時的なエラー）:** workflow を再実行すると、再ビルド・再デプロイ・全削除がやり直され、最終的にデプロイした分の 1 件だけが残ります。

## 15. 手動実行

GitHub 上の `Run workflow` はなくなりました。手動で実行するときは、次のコマンドを使います。

```bash
gcloud run jobs execute github-trend-digest --region="$REGION" --wait
gcloud run jobs executions list --job=github-trend-digest --region="$REGION" --limit=5
```

特定の言語だけを試したい場合は、`--args=rust` を付けます（定期実行では使いません）。

```bash
gcloud run jobs execute github-trend-digest --region="$REGION" --args=rust --wait
```

## 16. アプリ内の通知では拾えない失敗と、その気づき方

アプリは、次の場合に Discord へ失敗通知を送ります。

- 実行がエラーで終わったとき（終了コード 1）。メッセージ中の webhook URL・Gemini key・GitHub token は `[REDACTED]` に置き換えます。
- **Trending が 0 件**だったとき（`No trending repositories found (0 fetched)`）。GitHub の HTML 構造が変わってスクレイパーが 0 件になった場合も、ここに来ます。
- **全件で要約に失敗した**とき（`All N repositories failed to summarize (0/N succeeded)`）。

後ろの 2 つは、**終了コード 0 のまま**です。Cloud Run の executions 一覧では Succeeded と表示されるため、気づく手段は Discord の失敗通知だけです。

次の失敗は、アプリ内の通知が届きません。

| # | 失敗 | アプリ内の通知が届かない理由 | 気づき方 |
|---|---|---|---|
| 1 | コンテナが起動しない（イメージの pull 失敗、ENTRYPOINT の誤り） | アプリが動かない | 08:00 / 18:00 のまとめが届かない。`gcloud run jobs executions list --job=github-trend-digest --region=$REGION` で Failed になっている |
| 2 | Secret が読めない（IAM の不足、secret の無効化・削除） | コンテナが起動する前に失敗する | 1 と同じ。実行の詳細やログにある secret アクセスのエラー |
| 3 | task-timeout（30 分）を超える | SIGTERM / kill で強制終了される | まとめが届かない（または途中まで）。executions の状態が timeout |
| 4 | OOM | プロセスが kill される | 1 と同じ。Cloud Logging のメモリ超過ログ |
| 5 | Cloud Scheduler から起動できない（invoker の権限、API の無効化、Job の削除） | Job が実行されない | まとめが届かない。`gcloud scheduler jobs describe <name> --location=$REGION` の `status` / `lastAttemptTime`、Scheduler のログ |
| 6 | Discord の障害、webhook の失効 | 通知の送り先そのものが使えない | 何も届かない。Cloud Logging の `Failed to send failure notification` |
| 7 | 設定読み込みのエラー、ログ初期化の失敗 | webhook を確定する前に終了する | 1 と同じ（stderr に `Configuration error`） |
| 8 | panic | `finish` を通らない（exit 101） | 1 と同じ |
| 9 | 一部の要約だけが失敗する（1 件以上は成功。exit 0） | 0 件ではないので通知の対象外 | 届くまとめの件数がいつもより少ない。Cloud Logging の `Failed to create summary for …` の warn |

基本的な検知は、**人間が「決まった時刻にまとめが届かないこと」に気づく**ことです。確認用のコマンド:

```bash
gcloud run jobs executions list --job=github-trend-digest --region="$REGION" --limit=5
gcloud scheduler jobs describe github-trend-digest-0800 --location="$REGION" --format='value(status,lastAttemptTime)'
gcloud scheduler jobs describe github-trend-digest-1800 --location="$REGION" --format='value(status,lastAttemptTime)'
gcloud logging read 'resource.type="cloud_run_job" AND resource.labels.job_name="github-trend-digest" AND severity>=WARNING' \
  --limit=20 --freshness=1d
```

### 残存リスク（スコープ外）

アプリ内の失敗通知は、通知する本文と `finish` の中のログから秘匿値を伏せます。しかし、既存の infrastructure がもともと出しているログ
（例: `discord.rs` の `Network error sending Discord notification: ...`。reqwest のエラーに webhook URL が含まれうる）は、伏せていません。
Cloud Logging には secrets を自動で隠す仕組みがないため、ログの閲覧権限は必要な人だけに絞ってください。

## 17. ロールバック

古いイメージは残らない（デプロイした 1 件だけ）ので、イメージを指定して戻すことはできません。

```text
git revert <commit> → PR → main へのマージ → deploy workflow による再ビルドと再デプロイ
```

`gcloud run jobs update --image IMAGE@<前の digest>` は使えません（前の digest はすでに削除されています）。
