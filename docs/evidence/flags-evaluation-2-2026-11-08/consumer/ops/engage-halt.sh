#!/usr/bin/env bash
# Engage the `checkout_halt` kill switch with Batter's `batter-flags` operator
# command, then confirm that every Live checkout-service process applied it.
#
#   BATTER_FLAGS_DATABASE_URL='postgres://operator@db.internal/checkout?sslmode=verify-full' \
#     ops/engage-halt.sh --actor "$USER" --reason "contain duplicate charges" [--wait 60]
#
# While the switch is engaged the `checkout` route group answers 503 `disabled`
# and claimed `receipt.send` jobs are continued later without consuming attempts.
#
# The procedure performs at most one write. It reads the switch's revision N,
# then writes `kill checkout_halt engage --if-revision N`, never `--yes`. When
# the command cannot tell whether that write committed (exit 4, a panic, a
# signal, or an unreadable report), it reads `history` and then re-issues the
# same write with the same `--if-revision N`. The store answers a request that
# is already satisfied with `unchanged` before it compares revisions, and
# refuses any other write since N with `revision_mismatch`, so the re-issue
# writes only if the first attempt did not commit and nothing else changed the
# switch. A release by someone else after our write is therefore never undone.
#
# Exit status:
#   0   engaged, and every Live process reports a version at or past the write
#   10  engaged in the store, but the rollout is not confirmed (no Live process,
#       still pending after --wait, the fleet unreadable, or no Live declarer);
#       follow up with `batter-flags fleet --version V`
#   20  not engaged by this run; nothing was written (or a later write released
#       it). Safe to re-run after reading the message.
#   30  the write's outcome is still unknown after reconciliation; run
#       `batter-flags history checkout_halt` before doing anything else
#   2   usage or configuration error; nothing was contacted or written
#
# If the store cannot take the write (exit 20 with `store_behind_fleet`, or a
# full NOTIFY queue, SQLSTATE 54000), contain with the settings floor instead:
# set FLAG_CHECKOUT_HALT=engaged for the service and restart it.
#
# Environment: BATTER_FLAGS_DATABASE_URL (required; never printed),
# BATTER_FLAGS_BIN (default `batter-flags`), ENGAGE_HALT_TIMEOUT (seconds per
# command, 1-60, default 10), ENGAGE_HALT_POLL_SECONDS (default 2). No `PG*`
# variable may be set; the command refuses them.

set -uo pipefail

readonly SWITCH=checkout_halt
readonly FLAGS_BIN="${BATTER_FLAGS_BIN:-batter-flags}"
readonly TIMEOUT="${ENGAGE_HALT_TIMEOUT:-10}"
readonly POLL="${ENGAGE_HALT_POLL_SECONDS:-2}"
readonly MAX_REISSUES=3

readonly EXIT_ENGAGED=0 EXIT_USAGE=2 EXIT_UNCONFIRMED=10 EXIT_NOT_ENGAGED=20 EXIT_UNKNOWN=30

ACTOR=""
REASON=""
WAIT=60
CLI_OUT=""
CLI_RC=0

say() { printf 'engage-halt: %s\n' "$*" >&2; }

usage() {
    say "usage: ops/engage-halt.sh --actor NAME --reason TEXT [--wait SECONDS]"
    exit "$EXIT_USAGE"
}

while (($# > 0)); do
    case "$1" in
        --actor) (($# >= 2)) || usage; ACTOR="$2"; shift 2 ;;
        --reason) (($# >= 2)) || usage; REASON="$2"; shift 2 ;;
        --wait) (($# >= 2)) || usage; WAIT="$2"; shift 2 ;;
        *) usage ;;
    esac
done
[[ -n "$ACTOR" && -n "$REASON" ]] || usage
[[ "$WAIT" =~ ^[0-9]+$ ]] || usage
command -v jq >/dev/null 2>&1 || { say "jq is required"; exit "$EXIT_USAGE"; }
command -v "$FLAGS_BIN" >/dev/null 2>&1 || { say "operator command not found: $FLAGS_BIN"; exit "$EXIT_USAGE"; }
[[ -n "${BATTER_FLAGS_DATABASE_URL:-}" ]] || { say "BATTER_FLAGS_DATABASE_URL is not set"; exit "$EXIT_USAGE"; }

# Runs one operator command with a JSON report. Every exit writes exactly one
# document, except the argument parser's own (exit 2) and a panic (101).
cli() {
    CLI_OUT="$("$FLAGS_BIN" --timeout "$TIMEOUT" --output json "$@")"
    CLI_RC=$?
}

has_report() { printf '%s' "$CLI_OUT" | jq -e 'type == "object"' >/dev/null 2>&1; }

# Reads a field of the last report; prints `null` when it or the report is absent.
field() {
    if has_report; then
        printf '%s' "$CLI_OUT" | jq -r "$1" 2>/dev/null || printf 'null'
    else
        printf 'null'
    fi
}

relay() {
    local message
    message="$(field '.message')"
    [[ "$message" != null ]] && say "batter-flags: $message"
    printf '%s' "$CLI_OUT" | jq -r '.warnings[]? | "engage-halt: warning: " + .' 2>/dev/null >&2
}

# Reads the switch (read-only): sets REVISION and STATE (engaged, released or
# absent). Returns 1, with a reason, when it cannot.
read_switch() {
    cli show "$SWITCH"
    if [[ "$CLI_RC" -eq 2 ]]; then
        relay
        say "the operator command refused its configuration; nothing written"
        exit "$EXIT_USAGE"
    fi
    if [[ "$CLI_RC" -ne 0 ]]; then
        relay
        say "could not read $SWITCH (exit $CLI_RC)"
        return 1
    fi
    REVISION="$(field '.data.revision')"
    if [[ ! "$REVISION" =~ ^[0-9]+$ ]]; then
        say "the show report has no revision"
        return 1
    fi
    STATE="$(field '.data.state.switch')"
    [[ "$STATE" == null ]] && STATE=absent
    return 0
}

# The checks a kill would fail anyway, made before anything is written.
require_writable() {
    if [[ "$(field '.data.declaration.declared')" != true ]]; then
        say "no retained catalog declares $SWITCH, so a kill would be refused; nothing written"
        exit "$EXIT_NOT_ENGAGED"
    fi
    if [[ "$(field '.data.stored')" == archived ]]; then
        say "$SWITCH is archived; restore it with \`batter-flags restore\` first; nothing written"
        exit "$EXIT_NOT_ENGAGED"
    fi
}

# The one guarded write. Sets RESULT and, where the report says, VERSION and PROPOSED.
kill_guarded() {
    cli kill "$SWITCH" engage --actor "$ACTOR" --reason "$REASON" --if-revision "$1"
    relay
    VERSION=""
    PROPOSED=""
    local outcome
    outcome="$(field '.outcome')"
    case "$CLI_RC" in
        0)
            case "$outcome" in
                written) RESULT=written; VERSION="$(field '.data.version')" ;;
                written_undeclared) RESULT=written_undeclared; VERSION="$(field '.data.version')" ;;
                unchanged) RESULT=unchanged; VERSION="$(field '.data.revision')" ;;
                # Success with an outcome this procedure does not know: do not guess.
                *) RESULT=unknown ;;
            esac ;;
        1) RESULT=not_written ;;
        2) RESULT=usage ;;
        3) if [[ "$outcome" == revision_mismatch ]]; then RESULT=mismatch; else RESULT=refused; fi ;;
        4) RESULT=unknown; PROPOSED="$(field '.data.proposed_version')" ;;
        # A panic (101), a signal or anything else: the COMMIT may have been sent.
        *) RESULT=unknown ;;
    esac
    if [[ "$RESULT" == unknown ]] && ! has_report; then
        say "the operator command exited $CLI_RC without a report"
    fi
}

# Did the write proposed at version $1 commit? Prints yes, no or unreadable.
committed_at() {
    [[ "$1" =~ ^[0-9]+$ ]] || { printf 'unreadable'; return; }
    cli history "$SWITCH"
    if [[ "$CLI_RC" -ne 0 ]] || ! has_report; then printf 'unreadable'; return; fi
    printf '%s' "$CLI_OUT" | jq -e --argjson v "$1" --arg actor "$ACTOR" \
        '[.data[]? | select(.version == $v and .actor == $actor and .after.state.switch == "engaged")] | length > 0' \
        >/dev/null 2>&1 && printf 'yes' || printf 'no'
}

# Waits until every Live process applied version $1. Exits the script.
confirm_rollout() {
    local version="$1" deadline=$((SECONDS + WAIT)) reached live
    if ! read_switch; then
        say "engaged at version $version, but the switch could not be re-read"
        exit "$EXIT_UNCONFIRMED"
    fi
    if [[ "$STATE" != engaged ]]; then
        say "$SWITCH is $STATE at revision $REVISION: a later write released it; review \`batter-flags history $SWITCH\`"
        exit "$EXIT_NOT_ENGAGED"
    fi
    while :; do
        cli fleet --version "$version"
        if [[ "$CLI_RC" -ne 0 ]]; then
            relay
            say "engaged at version $version, but the fleet could not be read (exit $CLI_RC)"
            exit "$EXIT_UNCONFIRMED"
        fi
        reached="$(field '.data.rollout.reached')"
        live="$(field '.data.rollout.live')"
        if [[ "$reached" == true ]]; then
            say "engaged at version $version and applied by all $live Live processes"
            exit "$EXIT_ENGAGED"
        fi
        if [[ "$live" == 0 ]]; then
            say "engaged at version $version, but no process is Live to apply it"
            exit "$EXIT_UNCONFIRMED"
        fi
        if ((SECONDS >= deadline)); then
            relay
            say "engaged at version $version; not yet applied by: $(field '[.data.rollout.behind[]?] | join(", ")')"
            exit "$EXIT_UNCONFIRMED"
        fi
        sleep "$POLL"
    done
}

refusal_advice() {
    local refusal
    refusal="$(field '.data.refusal')"
    case "$refusal" in
        store_behind_fleet)
            say "the flags database is behind the fleet (a restore or failover); follow the restore runbook, and contain now with FLAG_CHECKOUT_HALT=engaged and a restart" ;;
        undeclared) say "no retained catalog declares $SWITCH; deploy a build that declares it, or use FLAG_CHECKOUT_HALT=engaged" ;;
        archived) say "$SWITCH is archived; restore it with \`batter-flags restore\` first" ;;
        unknown_row_encoding) say "a newer build wrote $SWITCH; use a newer batter-flags" ;;
        *) say "refused: $refusal" ;;
    esac
}

# A revision mismatch: something wrote the switch since revision N.
after_mismatch() {
    if ! read_switch; then
        say "$SWITCH changed since revision $1 and could not be re-read; nothing written by this run's last attempt"
        exit "$EXIT_NOT_ENGAGED"
    fi
    if [[ "$STATE" == engaged ]]; then
        say "$SWITCH was engaged by another write (revision $REVISION)"
        confirm_rollout "$REVISION"
    fi
    say "$SWITCH changed since revision $1 and is now $STATE at revision $REVISION; not retrying over another operator's write. Review \`batter-flags history $SWITCH\`, then re-run"
    exit "$EXIT_NOT_ENGAGED"
}

read_switch || { say "nothing written"; exit "$EXIT_NOT_ENGAGED"; }
require_writable
if [[ "$STATE" == engaged ]]; then
    say "$SWITCH is already engaged at revision $REVISION"
    confirm_rollout "$REVISION"
fi
readonly GUARD="$REVISION"
say "engaging $SWITCH from revision $GUARD"
kill_guarded "$GUARD"

reissues=0
while [[ "$RESULT" == unknown ]]; do
    if [[ -n "$PROPOSED" ]]; then
        case "$(committed_at "$PROPOSED")" in
            yes)
                say "history shows this run's write committed at version $PROPOSED"
                RESULT=written
                VERSION="$PROPOSED"
                break ;;
            no) say "history has no write at version $PROPOSED yet" ;;
            unreadable) say "history could not be read" ;;
        esac
    fi
    if ((reissues >= MAX_REISSUES)); then
        say "the outcome of engaging $SWITCH is still unknown after $reissues re-issues; run \`batter-flags history $SWITCH\` and \`batter-flags show $SWITCH\` before anything else"
        exit "$EXIT_UNKNOWN"
    fi
    reissues=$((reissues + 1))
    say "re-issuing the same guarded write (--if-revision $GUARD); it writes only if the first attempt did not commit"
    kill_guarded "$GUARD"
done

case "$RESULT" in
    written | unchanged) confirm_rollout "$VERSION" ;;
    written_undeclared)
        say "written at version $VERSION, but no Live process declares $SWITCH, so nothing applies it yet"
        exit "$EXIT_UNCONFIRMED" ;;
    mismatch) after_mismatch "$GUARD" ;;
    refused) refusal_advice; exit "$EXIT_NOT_ENGAGED" ;;
    not_written)
        if [[ "$(field '.data.sqlstate')" == 54000 ]]; then
            say "the NOTIFY queue is full, so every audited write fails; contain with FLAG_CHECKOUT_HALT=engaged and a restart"
        fi
        say "nothing was written; safe to re-run"
        exit "$EXIT_NOT_ENGAGED" ;;
    usage) say "the operator command refused its arguments or configuration; nothing written"; exit "$EXIT_USAGE" ;;
esac
say "unhandled result $RESULT"
exit "$EXIT_UNKNOWN"
