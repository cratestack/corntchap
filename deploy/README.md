# Deploying corntchap on Kubernetes

`deploy/k8s` is a kustomize base: a CloudNativePG `Cluster` (2 instances) with a daily `ScheduledBackup`,
a 2-replica `Deployment`, `Service` and an ingress-nginx `Ingress`.

Prerequisites: the [CloudNativePG operator](https://cloudnative-pg.io), ingress-nginx, and (optionally) cert-manager.

## Configure

1. Replace `corntchap.example.com` in `deployment.yaml` (`PUBLIC_URL`, as `https://HOST`) and `ingress.yaml`.
2. Backups: `cluster.yaml` points at an S3 bucket (`destinationPath`, secret `corntchap-backup-s3`). Edit it for your
   object store, or remove the `backup` block and `scheduledbackup.yaml` if you do not want backups.
3. TLS: the Ingress expects cert-manager with a `letsencrypt` ClusterIssuer. Otherwise drop the annotation and
   create the `corntchap-tls` secret yourself.
4. Pin the image tag in `kustomization.yaml` (`images:`) to a release or `sha-<commit>`.

## Keycloak (OIDC) client

Humans sign in through any OpenID provider; Keycloak is the reference. In your realm create a confidential client:

- Client ID: `corntchap`, Client authentication: on, Standard flow: on
- Valid redirect URIs: `https://HOST/auth/oidc/callback`
- Set `OIDC_ISSUER` in `deployment.yaml` to the realm URL (`https://KEYCLOAK/realms/<realm>`).

Users are matched by the `preferred_username` claim (then pinned to the OIDC subject on first sign-in), so disable
"Edit username" for users in the realm and only provision usernames you control (`corntchap-cli user add <username>`).

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
