#!/usr/bin/env bash

PATH_SCRIPTS="./scripts"

MOD=$(grep "module" go.mod)
MOD="${MOD/module /}"

EXCLUDE=""
while read n; do
  EXCLUDE+=$"$MOD/$n|"
done < ./coverage/exclude

EXCLUDE=${EXCLUDE%?};

go test -coverprofile=$PATH_SCRIPTS/coverage.out -coverpkg ./... ./...
grep -vwE "($EXCLUDE)" $PATH_SCRIPTS/coverage.out > $PATH_SCRIPTS/coverage-final.out
go tool cover -func=$PATH_SCRIPTS/coverage-final.out

rm $PATH_SCRIPTS/coverage.out
rm $PATH_SCRIPTS/coverage-final.out