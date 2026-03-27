#!/usr/bin/env bash
# scripts/restore.sh — Restore the SorobanPulse PostgreSQL database from a dump.
#
# Usage:
#   ./scripts/restore.sh ./backups/soroban_pulse_20260314T000000Z.dump
#   ./scripts/restore.sh s3://my-bucket/backups/soroban_pulse_20260314T000000Z.dump
#
# Environment variables:
#   DATABASE_URL — PostgreSQL connection string (required)
#
# WARNING: This will DROP and recreate the target database. All existing data
#          will be permanently lost. Confirm before running in production.
#
# Dependencies: pg_restore, psql, aws CLI (only for S3 sources)

set -euo pipefail

DATABASE_URL="${DATABASE_URL:?DATABASE_URL must be set}"
DUMP_SOURCE="${1:?Usage: $0 <dump-file-or-s3-uri>}"
LOCAL_DUMP="${DUMP_SOURCE}"

if [[ "${DUMP_SOURCE}" == s3://* ]]; then
    LOCAL_DUMP="/tmp/soroban_pulse_restore_$$.dump"
    echo "[restore] Downloading ${DUMP_SOURCE} → ${LOCAL_DUMP}"
    aws s3 cp "${DUMP_SOURCE}" "${LOCAL_DUMP}"
    echo "[restore] Download complete"
fi

echo "[restore] WARNING: This will overwrite all data in the target database."
read -r -p "[restore] Type 'yes' to continue: " CONFIRM
if [[ "${CONFIRM}" != "yes" ]]; then
    echo "[restore] Aborted."
    exit 1
fi

echo "[restore] Restoring from ${LOCAL_DUMP}"
pg_restore --clean --if-exists --no-owner --no-privileges \
    --dbname "${DATABASE_URL}" "${LOCAL_DUMP}"

echo "[restore] Restore complete"

if [[ "${DUMP_SOURCE}" == s3://* ]]; then
    rm -f "${LOCAL_DUMP}"
    echo "[restore] Temp file removed"
fi
