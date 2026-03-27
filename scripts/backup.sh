#!/usr/bin/env bash
# scripts/backup.sh — Dump the SorobanPulse PostgreSQL database.
#
# Usage:
#   ./scripts/backup.sh                  # dump to ./backups/
#   BACKUP_DEST=s3://my-bucket/backups ./scripts/backup.sh
#
# Environment variables:
#   DATABASE_URL   — PostgreSQL connection string (required)
#   BACKUP_DEST    — local directory or s3://bucket/prefix (default: ./backups)
#   KEEP_LOCAL     — keep local dump file after S3 upload (default: false)
#
# Dependencies: pg_dump, aws CLI (only for S3 uploads)

set -euo pipefail

DATABASE_URL="${DATABASE_URL:?DATABASE_URL must be set}"
BACKUP_DEST="${BACKUP_DEST:-./backups}"
KEEP_LOCAL="${KEEP_LOCAL:-false}"
TIMESTAMP="$(date -u +%Y%m%dT%H%M%SZ)"
FILENAME="soroban_pulse_${TIMESTAMP}.dump"
LOCAL_PATH="/tmp/${FILENAME}"

echo "[backup] Starting pg_dump → ${LOCAL_PATH}"
pg_dump --format=custom --no-password "${DATABASE_URL}" --file "${LOCAL_PATH}"
echo "[backup] Dump complete: $(du -sh "${LOCAL_PATH}" | cut -f1)"

if [[ "${BACKUP_DEST}" == s3://* ]]; then
    echo "[backup] Uploading to ${BACKUP_DEST}/${FILENAME}"
    aws s3 cp "${LOCAL_PATH}" "${BACKUP_DEST}/${FILENAME}"
    echo "[backup] Upload complete"
    if [[ "${KEEP_LOCAL}" != "true" ]]; then
        rm -f "${LOCAL_PATH}"
        echo "[backup] Local temp file removed"
    fi
else
    mkdir -p "${BACKUP_DEST}"
    mv "${LOCAL_PATH}" "${BACKUP_DEST}/${FILENAME}"
    echo "[backup] Saved to ${BACKUP_DEST}/${FILENAME}"
fi

echo "[backup] Done"
