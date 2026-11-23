# Deploying corntchap on Kubernetes

`deploy/k8s` is a kustomize base: a CloudNativePG `Cluster` (2 instances) with a daily `ScheduledBackup`,
a 2-replica `Deployment`, `Service` and an ingress-nginx `Ingress`.

Prerequisites:

- the [CloudNativePG operator](https://cloudnative-pg.io) (1.26 or newer, for CNPG-I plugin support) and ingress-nginx;
- cert-manager (required by the Barman Cloud plugin, which is installed through it, and optional otherwise for TLS);
- the [Barman Cloud plugin](https://cloudnative-pg.io/plugin-barman-cloud/) in the CNPG operator's namespace
  (usually `cnpg-system`), installed once per cluster: take the manifest from its
  [releases page](https://github.com/cloudnative-pg/plugin-barman-cloud/releases) and
  `kubectl apply -f manifest.yaml`. Skip it only if you remove the backup resources (see Configure).

## Configure

1. Replace `corntchap.example.com` in `deployment.yaml` (`PUBLIC_URL`, as `https://HOST`) and `ingress.yaml`.
2. Backups: `cluster.yaml` archives WAL and base backups through the Barman Cloud plugin; the `ObjectStore`
   `corntchap-backups` in the same file holds the S3 settings (`destinationPath`, secret `corntchap-backup-s3`,
   14-day retention). Edit it for your object store, or remove the `plugins` block, the `ObjectStore` and
   `scheduledbackup.yaml` (and its line in `kustomization.yaml`) if you do not want backups.
3. TLS: the Ingress expects cert-manager with a `letsencrypt` ClusterIssuer. Otherwise drop the annotation and
   create the `corntchap-tls` secret yourself.
4. Pin the image tag in `kustomization.yaml` (`images:`) to a release or `sha-<commit>`. Note: git release tags are
   `v0.1.0`, but the published image tags drop the `v` (`0.1.0`, `0.1`), so use `newTag: 0.1.0`, not `v0.1.0`.
   The image is multi-arch (amd64 and arm64).

## Files (optional)

Uploads (screenshots, logs, build artifacts, up to 1 GiB each) live in any S3-compatible store; RustFS is used in
development. Set `S3_ENDPOINT` (how the server reaches it), `S3_PUBLIC_ENDPOINT` (how browsers and agents reach it: the
presigned upload and download URLs point there; defaults to `S3_ENDPOINT`), `S3_BUCKET`, `S3_ACCESS_KEY`,
`S3_SECRET_KEY` (secret `corntchap-s3`), and optionally `S3_PREFIX` (put in front of every object key, e.g.
`corntchap/files/` for a bucket shared with other data; default the bucket root; `.` and `..` segments stop the
server), `S3_REGION` (default `us-east-1`), `S3_PATH_STYLE`
(default on; `0` for virtual-hosted buckets), `S3_CREATE_BUCKET=1` (create it on start), `FILES_MAX_BYTES` (default
1 GiB) and `FILES_INLINE_MAX_BYTES` (base64 uploads over MCP, default 5 MiB). Without `S3_ENDPOINT` the file tools
answer that uploads are not configured.

The prefix is part of each file's stored key, so changing `S3_PREFIX` only moves new uploads. To move old files too, copy
the objects under the new prefix, then `update files set storage_key = '<new prefix>' || storage_key` (strip the old
prefix first if there was one). The prefix keeps corntchap's own writes and deletes inside it, but it is not access
control: with a key that reaches the whole bucket, a leaked key reaches the rest of the bucket too. Where the store
supports it, limit the key to `<prefix>*` with a bucket policy.

Every file has a permanent link, `$PUBLIC_URL/files/PAY-F12/name`, that only works behind corntchap's login (bearer token
or console session); it redirects to a short-lived presigned URL. The bucket itself stays private. The console shows
image previews from `S3_PUBLIC_ENDPOINT`'s origin, which is added to its Content-Security-Policy.

## Keycloak (OIDC) client

Humans sign in through any OpenID provider; Keycloak is the reference. In your realm create a confidential client:

- Client ID: `corntchap`, Client authentication: on, Standard flow: on
- Valid redirect URIs: `https://HOST/auth/oidc/callback`
- Set `OIDC_ISSUER` in `deployment.yaml` to the realm URL (`https://KEYCLOAK/realms/<realm>`).

A person is identified by the OIDC issuer and subject (`sub`) together and nothing else (the issuer is `OIDC_PUBLIC_ISSUER`, else `OIDC_ISSUER`; people who signed in before issuers were recorded are adopted by the issuer configured at the next start, logged with a count; changing the issuer later makes everyone a new account): the first sign-in of an unknown `sub` creates the account
with a random kitchen handle (`saffron-okra-42`) that has nothing to do with the identity. Neither `preferred_username` nor
an e-mail address ever links or claims an account, so the realm's username and e-mail settings are not a trust boundary
here. A handle once used by an account is never given to a different account (a renamed-away name stays reserved to its first holder). The ID token's `name` (else a *verified* `email`) becomes the display name, refreshed at every sign-in and used only for showing
people; the handle is what others `@mention` and what admins type. People change their own handle with
`corntchap-cli me rename <handle>` or in the console.

Admins come from the IdP: add a realm role named `corntchap-admin` (override with `OIDC_ADMIN_ROLE`) and a client mapper
"User Realm Role" that puts roles into the ID token (claim `realm_access.roles`, add to ID token). The flag is synced on
every sign-in; a top-level `roles` or `groups` array also works. Admins made with `corntchap init` or by an
admin (`corntchap-cli user admin <handle> --on`) are never demoted by the IdP. Set `OIDC_PUBLIC_ISSUER` when browsers reach Keycloak under a different
URL than the server does (it is used for the IdP logout redirect). Browser logout (`POST /logout`) also ends the
Keycloak session.

**Login theme.** [cratestack/corntchap-keycloak](https://github.com/cratestack/corntchap-keycloak) styles Keycloak's
sign-in pages like corntchap's own (indigo band, cream card, light and dark, phone layout). Copy its jar into
`/opt/keycloak/providers/`, run `kc.sh build`, and pick **Login theme: `corntchap`** under Realm settings → Themes.
The dev stack mounts it from a checkout (`just keycloak-theme`, part of `just setup`).

Store the client credentials (the Deployment reads `client-id` and `client-secret` from this secret):

```sh
kubectl create namespace corntchap
kubectl -n corntchap create secret generic corntchap-oidc \
  --from-literal=client-id=... --from-literal=client-secret=...
kubectl -n corntchap create secret generic corntchap-backup-s3 \
  --from-literal=ACCESS_KEY_ID=... --from-literal=ACCESS_SECRET_KEY=...
```

## Apply

```sh
kubectl apply -k deploy/k8s
kubectl -n corntchap get cluster,pods
```

The Deployment reads `DATABASE_URL` from the `corntchap-db-app` secret (key `uri`) that CNPG creates for the
`corntchap-db` cluster. `AUTO_MIGRATE=1` applies the schema on startup.

## First run: create the admin

Once the pods are ready, create the local admin (not linked to any sign-in; it is the CLI's bootstrap identity) and a
7-day bootstrap token. It prints the admin's handle (random, or `--name <handle>`):

```sh
kubectl -n corntchap exec deploy/corntchap -- /corntchap init
```

Re-running `init` reuses the init admin and mints another 7-day token; older tokens stay valid until they expire. `init --name <x>`
refuses a person who signs in through the IdP. Humans who sign in through Keycloak are separate accounts; give them admin with the `corntchap-admin` role or
`corntchap-cli user admin <handle> --on`.

The image is distroless, so `exec` runs the binary directly (no shell). Then sign in from a workstation:

```sh
corntchap-cli login https://HOST --token <bootstrap-token>
```

(or `corntchap-cli login https://HOST` for the browser flow). `corntchap token --name <user> --hours N`
mints further tokens the same way.

## Cloud agents and network allowlists

Agents run in cloud sandboxes with restricted egress. Add `HOST` (the hostname only, HTTPS/443) to the allowlist
of every such environment (for example the allowed hosts of a Claude Code cloud environment) so agents can reach
`/mcp` and the REST API. Relays need `HOST` as well.

## Notes

- The Ingress disables proxy buffering and raises read/send timeouts to one hour; without this, SSE on `/mcp` and
  `/api/v1/relay/stream` is cut or delayed.
- The server refuses non-loopback requests not marked `X-Forwarded-Proto: https`; ingress-nginx sets it, but
  keep `PUBLIC_URL` on `https://`.
