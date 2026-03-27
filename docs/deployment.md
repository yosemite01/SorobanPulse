# Deployment Guide

## Environment Configuration

Soroban Pulse ships three `.env.*.example` templates:

| File | Purpose |
|------|---------|
| `.env.example` | Local development defaults |
| `.env.staging.example` | Staging environment — testnet, JSON logs, restricted CORS |
| `.env.production.example` | Production — mainnet, strict CORS, higher pool sizing |

Copy the appropriate template and fill in real values:

```bash
cp .env.staging.example .env.staging
cp .env.production.example .env.production
```

### Environment-specific behaviour (`ENVIRONMENT`)

Set the `ENVIRONMENT` variable to one of `development`, `staging`, or `production`.

| Behaviour | development | staging | production |
|-----------|-------------|---------|------------|
| `ALLOWED_ORIGINS=*` allowed | ✅ | ❌ panics at startup | ❌ panics at startup |
| `RUST_LOG_FORMAT` default | `text` | `json` | `json` |
| Recommended `API_KEY` | optional | required | required |

In staging and production, setting `ALLOWED_ORIGINS=*` will cause the service to **panic at startup** — you must list explicit origins.

### Key differences between environments

| Variable | Development | Staging | Production |
|----------|-------------|---------|------------|
| `STELLAR_RPC_URL` | testnet | testnet | mainnet |
| `ALLOWED_ORIGINS` | `*` | `https://staging.example.com` | `https://app.example.com,...` |
| `RUST_LOG` | `debug` | `info` | `warn` |
| `RATE_LIMIT_PER_MINUTE` | `60` | `60` | `30` |
| `DB_MAX_CONNECTIONS` | `10` | `10` | `20` |
| `BEHIND_PROXY` | `false` | `true` | `true` |

---

## Secret Management

Secrets (database password, API key, RPC URL) should never be stored in plain `.env` files in production. Use one of the following patterns.

### Docker Secrets (recommended for Docker Compose / Swarm)

Mount the secret as a file and point `DATABASE_URL_FILE` at it:

```yaml
# docker-compose.yml
services:
  app:
    environment:
      DATABASE_URL_FILE: /run/secrets/database_url
    secrets:
      - database_url

secrets:
  database_url:
    file: ./secrets/database_url.txt
```

When `DATABASE_URL_FILE` is set it takes precedence over `DATABASE_URL`. The file is read once at startup and its contents are trimmed of whitespace.

### Kubernetes Secrets

Create a secret and mount it as an environment variable:

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: soroban-pulse-secrets
stringData:
  DATABASE_URL: "postgres://user:pass@host:5432/db"
  API_KEY: "your-api-key"
---
# In your Deployment spec:
envFrom:
  - secretRef:
      name: soroban-pulse-secrets
```

Or mount as a file and use `DATABASE_URL_FILE`:

```yaml
volumes:
  - name: db-secret
    secret:
      secretName: soroban-pulse-secrets
volumeMounts:
  - name: db-secret
    mountPath: /run/secrets
    readOnly: true
env:
  - name: DATABASE_URL_FILE
    value: /run/secrets/DATABASE_URL
```

### AWS Secrets Manager

Use the [AWS Secrets Manager Agent](https://docs.aws.amazon.com/secretsmanager/latest/userguide/secrets-manager-agent.html) or an init container to write the secret to a file, then set `DATABASE_URL_FILE` to that path. Alternatively, use the [External Secrets Operator](https://external-secrets.io/) to sync secrets into Kubernetes Secrets automatically.

### HashiCorp Vault

Use the [Vault Agent Injector](https://developer.hashicorp.com/vault/docs/platform/k8s/injector) to render secrets into a file at `/vault/secrets/database_url`, then:

```bash
DATABASE_URL_FILE=/vault/secrets/database_url
```

### Secret hygiene

- No secrets are logged at any log level. The `DATABASE_URL` is consumed at startup and never emitted to logs. The `API_KEY` is stored in memory only and never traced.
- Rotate secrets by updating the secret store and restarting the service (or using a sidecar that signals the process).

---

## TLS Termination

Soroban Pulse speaks plain HTTP and **must never be exposed directly on port 80 or 443 without TLS in front of it**. All TLS termination must happen at a reverse proxy or load balancer layer.

Set `BEHIND_PROXY=true` in your environment so the service trusts `X-Forwarded-For` headers from the proxy and logs real client IPs.

---

### Option 1 — nginx (self-managed)

Install certbot and obtain a certificate, then use the config below.

```nginx
# /etc/nginx/sites-available/soroban-pulse
server {
    listen 80;
    server_name api.example.com;
    return 301 https://$host$request_uri;
}

server {
    listen 443 ssl http2;
    server_name api.example.com;

    ssl_certificate     /etc/letsencrypt/live/api.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/api.example.com/privkey.pem;
    ssl_protocols       TLSv1.2 TLSv1.3;
    ssl_ciphers         HIGH:!aNULL:!MD5;

    location / {
        proxy_pass         http://127.0.0.1:3000;
        proxy_set_header   Host              $host;
        proxy_set_header   X-Real-IP         $remote_addr;
        proxy_set_header   X-Forwarded-For   $proxy_add_x_forwarded_for;
        proxy_set_header   X-Forwarded-Proto $scheme;
    }
}
```

```bash
sudo ln -s /etc/nginx/sites-available/soroban-pulse /etc/nginx/sites-enabled/
sudo nginx -t && sudo systemctl reload nginx
```

---

### Option 2 — Caddy (automatic HTTPS)

```caddyfile
# /etc/caddy/Caddyfile
api.example.com {
    reverse_proxy localhost:3000
}
```

```bash
sudo systemctl reload caddy
```

---

### Option 3 — AWS Application Load Balancer (ALB)

1. Create an ALB with an HTTPS listener on port 443.
2. Attach an ACM certificate to the listener.
3. Add a target group pointing to the EC2/ECS instance on port 3000.
4. Set the security group to allow inbound 443 from the internet and inbound 3000 **only from the ALB security group**.
5. Set `BEHIND_PROXY=true` so ALB-injected `X-Forwarded-For` headers are trusted.

---

## Database Backup and Recovery

### RTO / RPO targets

| Target | Goal |
|--------|------|
| RPO (Recovery Point Objective) | ≤ 1 hour (with hourly `pg_dump` schedule) |
| RTO (Recovery Time Objective) | ≤ 30 minutes (restore from latest dump) |

For stricter RPO, enable WAL archiving (see below).

### pg_dump schedule (recommended for most deployments)

Use `scripts/backup.sh` to create a compressed custom-format dump:

```bash
# Dump to a local directory
DATABASE_URL=postgres://user:pass@localhost/soroban_pulse ./scripts/backup.sh

# Dump and upload to S3
DATABASE_URL=postgres://... BACKUP_DEST=s3://my-bucket/soroban-pulse ./scripts/backup.sh
```

Schedule with cron (hourly example):

```cron
0 * * * * DATABASE_URL=postgres://... BACKUP_DEST=s3://my-bucket/backups /app/scripts/backup.sh >> /var/log/soroban-backup.log 2>&1
```

### Restoring from a dump

```bash
# From a local file
DATABASE_URL=postgres://... ./scripts/restore.sh ./backups/soroban_pulse_20260314T000000Z.dump

# From S3
DATABASE_URL=postgres://... ./scripts/restore.sh s3://my-bucket/backups/soroban_pulse_20260314T000000Z.dump
```

The restore script prompts for confirmation before overwriting data.

### WAL archiving (for sub-minute RPO)

Enable continuous archiving in `postgresql.conf`:

```ini
wal_level = replica
archive_mode = on
archive_command = 'aws s3 cp %p s3://my-bucket/wal/%f'
```

Use [pgBackRest](https://pgbackrest.org/) or [Barman](https://pgbarman.org/) for managed WAL archiving and point-in-time recovery.

### Managed database recommendations

For production workloads, prefer a managed PostgreSQL service to offload backup and HA concerns:

- **AWS RDS for PostgreSQL** — automated backups, Multi-AZ, point-in-time recovery up to 35 days.
- **Google Cloud SQL** — automated backups, read replicas, point-in-time recovery.
- **Supabase** — managed Postgres with daily backups on paid plans.

When using a managed service, disable the `db` service in `docker-compose.yml` and point `DATABASE_URL` at the managed endpoint.

### Testing backups against Docker Compose

```bash
# 1. Start the stack
docker-compose up -d db

# 2. Run a backup
DATABASE_URL=postgres://user:pass@localhost:5432/soroban_pulse \
  BACKUP_DEST=./backups ./scripts/backup.sh

# 3. Restore into a fresh database to verify
createdb soroban_pulse_verify
DATABASE_URL=postgres://user:pass@localhost:5432/soroban_pulse_verify \
  ./scripts/restore.sh ./backups/soroban_pulse_*.dump
```

---

## Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `ENVIRONMENT` | Deployment environment (`development`/`staging`/`production`) | `development` |
| `BEHIND_PROXY` | Trust `X-Forwarded-For` from upstream proxy/load balancer | `false` |
| `DATABASE_URL_FILE` | Path to a file containing the database URL (takes precedence over `DATABASE_URL`) | — |

See the root [README](../README.md) for all other variables.

---

## Security Checklist

- [ ] TLS termination is handled by nginx, Caddy, or a cloud load balancer
- [ ] Port 3000 is firewalled from public internet access
- [ ] `BEHIND_PROXY=true` is set when running behind a proxy
- [ ] Certificates are auto-renewed (certbot timer or Caddy/ACM managed)
- [ ] `ENVIRONMENT=production` is set in production
- [ ] `ALLOWED_ORIGINS` lists only known domains (no `*`)
- [ ] `API_KEY` is set and rotated regularly
- [ ] Secrets are managed via Docker Secrets, Kubernetes Secrets, or a vault — not plain `.env` files
- [ ] Database backups are scheduled and restore procedure is tested
