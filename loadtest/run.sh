#!/bin/bash
# Runs one k6 scenario of web.js against a compose project and samples docker stats.
# Usage: loadtest/run.sh <label> <realistic|stress|login> [extra docker -e args...]
# Env:   PROJECT (default test-api-load), results go to loadtest/results/.
set -u
dir=$(cd "$(dirname "$0")" && pwd)
project=${PROJECT:-test-api-load}
label=$1; scenario=$2; shift 2
mkdir -p "$dir/results"
rm -f "$dir/results/stats-$label.csv"
(
  while true; do
    docker stats --no-stream --format '{{.Name}},{{.CPUPerc}},{{.MemUsage}}' \
      "$project-app-1" "$project-db-1" | sed "s/^/$(date +%s),/" >> "$dir/results/stats-$label.csv"
    sleep 1
  done
) &
sampler=$!
docker run --rm --network "${project}_default" -v "$dir:/scripts" -e SCENARIO="$scenario" "$@" \
  grafana/k6:latest run --quiet \
  --summary-export "/scripts/results/$label.json" \
  /scripts/web.js | tee "$dir/results/$label.txt"
kill $sampler
