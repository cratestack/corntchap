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

## Keycloak (OIDC) client

Humans sign in through any OpenID provider; Keycloak is the reference. In your realm create a confidential client:

- Client ID: `corntchap`, Client authentication: on, Standard flow: on
- Valid redirect URIs: `https://HOST/auth/oidc/callback`
- Set `OIDC_ISSUER` in `deployment.yaml` to the realm URL (`https://KEYCLOAK/realms/<realm>`).

Users are created on first sign-in (just-in-time) with the name from `preferred_username` (or the local part of a
*verified* e-mail). A user pre-provisioned with `corntchap-cli user add <username>` is linked to the OIDC subject on
first sign-in. Because the username is trusted for that link, **disable "Edit username"** in the realm (Realm settings,
Login) so users cannot rename themselves to someone else's login.

Admins come from the IdP: add a realm role named `corntchap-admin` (override with `OIDC_ADMIN_ROLE`) and a client mapper
"User Realm Role" that puts roles into the ID token (claim `realm_access.roles`, add to ID token). The flag is synced on
every sign-in; a top-level `roles` or `groups` array also works. Admins made with `corntchap init --admin` or
`user add --admin` are never demoted by the IdP. Set `OIDC_PUBLIC_ISSUER` when browsers reach Keycloak under a different
URL than the server does (it is used for the IdP logout redirect). Browser logout (`POST /logout`) also ends the
Keycloak session.

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

Once the pods are ready, create the first admin (a Keycloak username) and a 7-day bootstrap token:

```sh
kubectl -n corntchap exec deploy/corntchap -- /corntchap init --admin <keycloak-username>
```

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
