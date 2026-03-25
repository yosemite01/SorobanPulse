# Soroban Pulse

A lightweight Rust backend service that indexes Soroban smart contract events on the Stellar network and exposes them via a REST API.

## Tech Stack

- **Rust** + **Axum** (web framework)
- **Tokio** (async runtime)
- **PostgreSQL** + **SQLx** (database + migrations)
- **Stellar Soroban RPC** (event source)

## Project Structure

```
src/
├── main.rs       # Entry point, wires everything together
├── config.rs     # Environment config
├── db.rs         # DB pool + migrations
├── models.rs     # Data types (Event, RPC response shapes)
├── indexer.rs    # Background event polling worker
├── routes.rs     # Axum router
├── handlers.rs   # Request handlers
└── error.rs      # Unified error type
migrations/
└── 20260314000000_create_events.sql
```

## Setup

### 1. Prerequisites

- Rust (stable)
- PostgreSQL 14+
- `sqlx-cli` (optional, for manual migrations)

### 2. Configure environment

Copy the provided `.env.example` template to a new file named `.env`:

```bash
cp .env.example .env
```

Open the newly created `.env` file in your editor and fill in your own real values. Be sure to replace the placeholder credentials (e.g., `<USER>`, `<PASSWORD>`) with your actual database and network details.

| Variable          | Description                          | Default                                  |
|-------------------|--------------------------------------|------------------------------------------|
| `DATABASE_URL`    | PostgreSQL connection string         | required                                 |
| `STELLAR_RPC_URL` | Soroban RPC endpoint                 | `https://soroban-testnet.stellar.org`    |
| `DB_MAX_CONNECTIONS` | Max number of connections in the Postgres pool | `10` |
| `DB_MIN_CONNECTIONS` | Min number of connections in the Postgres pool | `1` |
| `START_LEDGER`    | Ledger to start indexing from (0 = latest) | `0`                               |
| `PORT`            | HTTP server port                     | `3000`                                   |
| `API_KEY`         | Optional key for API authentication  | (disabled)                               |

> **Note on Authentication:** You can enable optional API key authentication by setting the `API_KEY` environment variable. When set, all requests (except `/health` and `/healthz/*` endpoints) will require either an `Authorization: Bearer <API_KEY>` or an `X-Api-Key: <API_KEY>` header. If `API_KEY` is unset or omitted from your configuration, authentication is bypassed and all requests pass through.

### 3. Run with Docker Compose (easiest)

```bash
docker-compose up --build
```

### 4. Run locally

```bash
# Start PostgreSQL, then:
cargo run
```

Migrations run automatically on startup.

## API

### `GET /health`
```json
{ "status": "ok" }
```

### `GET /events?page=1&limit=20`
Returns paginated events across all contracts.
```json
{
  "data": [
    {
      "id": "uuid",
      "contract_id": "CABC...",
      "event_type": "contract",
      "tx_hash": "abc123...",
      "ledger": 1234567,
      "timestamp": "2026-03-14T00:00:00Z",
      "event_data": { "value": {}, "topic": [] },
      "created_at": "2026-03-14T00:00:01Z"
    }
  ],
  "total": 100,
  "page": 1,
  "limit": 20
}
```

### `GET /events/{contract_id}`
Returns all events for a specific contract.

### `GET /events/tx/{tx_hash}`
Returns all events from a specific transaction. If nothing has been indexed for that hash yet (including valid on-chain transactions that emitted no Soroban events), the response is **200 OK** with an empty `"data"` array — not **404**.

## How It Works

1. On startup, the app connects to PostgreSQL and runs migrations.
2. A background Tokio task (`indexer.rs`) polls the Soroban RPC `getEvents` method in a loop.
3. New events are inserted with `ON CONFLICT DO NOTHING` to avoid duplicates.
4. The Axum HTTP server runs concurrently, serving queries against the indexed data.

## Notes

- The indexer polls every 5 seconds when no new ledgers are available, and 10 seconds on error.
- `START_LEDGER=0` automatically starts from the latest ledger at boot time.
- All endpoints return JSON. Errors include an `"error"` field with a description.

## Deployment

See [docs/deployment.md](docs/deployment.md) for TLS termination options (nginx, Caddy, AWS ALB) and production security guidance.
