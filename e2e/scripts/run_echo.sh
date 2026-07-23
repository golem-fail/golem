#!/usr/bin/env bash
# Fixture for e2e/run.test.toml — the `run` action executes this script
# (flow-dir-relative path) and captures stdout into save_to. printf (no trailing
# newline) keeps the captured value exact.
printf 'golem-run-99'
