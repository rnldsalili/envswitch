#!/usr/bin/env bash
set -Eeuo pipefail
umask 077
export LC_ALL=C
SELF=${BASH_SOURCE[0]:-}
STAGE='' LOCK='' TOKEN='' LOCKED=0 COMMITTED=0 NEW_PAYLOAD='' ACTIVE_CHILD='' WATCHDOG=''
PROFILE_BACKUP='' PROFILE_WRITTEN=0 PROFILE_EXISTED=0 PROFILE_PATH=''
J_PATH=() J_BACKUP=() J_CHANGED=() CREATED=() TEMPS=()
BEGIN='# >>> envswitch PATH >>>'; END='# <<< envswitch PATH <<<'
VERSION_RE='^v?0\.0\.(0|[1-9][0-9]*)$'
TARGETS=(aarch64-apple-darwin x86_64-apple-darwin x86_64-unknown-linux-musl aarch64-unknown-linux-musl x86_64-pc-windows-msvc)
info() { printf 'info: %s\n' "$1"; }
die() { local message; for message in "$@"; do printf 'error: %s\n' "$message" >&2; done; exit 1; }
unexpected() { [[ ${BASH_SUBSHELL:-0} -eq 0 ]] || return; trap - ERR; die 'envswitch installation failed: a required operation failed'; }
setting() { die "invalid installer setting: $1"; }
need() { command -v "$1" >/dev/null 2>&1 || die "required installer tool not found: $1"; }

# One portable JSON grammar: canonical UTF-8 hex strings keep NUL/control bytes out of shell values.
need cat
trap unexpected ERR
JSON_AWK=$(cat <<'JSON_AWK_END'
function bad() { failed=1; exit 1 }
function hex(n) { return sprintf("%02x",n) }
function utf(n) {
 if(n<128)return hex(n)
 if(n<2048)return hex(192+int(n/64)) hex(128+n%64)
 if(n<65536)return hex(224+int(n/4096)) hex(128+int(n/64)%64) hex(128+n%64)
 return hex(240+int(n/262144)) hex(128+int(n/4096)%64) hex(128+int(n/64)%64) hex(128+n%64)
}
function h4(  j,c,n) {
 n=0; for(j=0;j<4;j++){c=tolower(substr(text,pos++,1)); if(c!~/^[0-9a-f]$/)bad(); n=n*16+index("0123456789abcdef",c)-1} return n
}
function string(  c,n,low,result,b,need,min,j,cp) {
 if(substr(text,pos++,1)!="\"")bad(); result=""
 while(pos<=length(text)) {
  c=substr(text,pos++,1); n=ord[c]
  if(c=="\""){ value=result; return }
  if(c=="\\") {
   c=substr(text,pos++,1)
   if(c=="u") {
    n=h4()
    if(n>=55296 && n<=56319){if(substr(text,pos,2)!="\\u")bad();pos+=2;low=h4();if(low<56320||low>57343)bad();n=65536+(n-55296)*1024+low-56320}
    else if(n>=56320 && n<=57343)bad()
    result=result utf(n)
   } else if(c=="\""||c=="\\"||c=="/")result=result hex(ord[c])
   else if(c=="b")result=result "08"
   else if(c=="f")result=result "0c"
   else if(c=="n")result=result "0a"
   else if(c=="r")result=result "0d"
   else if(c=="t")result=result "09"
   else bad()
  } else {
   if(n<32)bad()
   if(n<128)result=result hex(n)
   else {
    if(n>=194&&n<=223){need=1;cp=n-192;min=128}
    else if(n>=224&&n<=239){need=2;cp=n-224;min=2048}
    else if(n>=240&&n<=244){need=3;cp=n-240;min=65536}
    else bad()
    result=result hex(n)
    for(j=0;j<need;j++){b=ord[substr(text,pos++,1)];if(b<128||b>191)bad();cp=cp*64+b-128;result=result hex(b)}
    if(cp<min||cp>1114111||(cp>=55296&&cp<=57343))bad()
   }
  }
 }
 bad()
}
function ws(){while(substr(text,pos,1)~/[ \t\r\n]/ && pos<=length(text))pos++}
function node(path,depth,  c,key,id,count,start,num) {
 if(depth>128)bad();ws();c=substr(text,pos,1)
 if(c=="{") {
  id=++objects;pos++;ws();count=0
  if(substr(text,pos,1)!="}")while(1){string();key=value;if((id SUBSEP key) in keys)bad();keys[id SUBSEP key]=1;ws();if(substr(text,pos++,1)!=":")bad();node(path "/" key,depth+1);count++;ws();c=substr(text,pos++,1);if(c=="}")break;if(c!=",")bad();ws()}
  else pos++
  print path "\tobject\t" count
 } else if(c=="[") {
  pos++;ws();count=0
  if(substr(text,pos,1)!="]")while(1){node(path "/" count,depth+1);count++;ws();c=substr(text,pos++,1);if(c=="]")break;if(c!=",")bad();ws()}
  else pos++
  print path "\tarray\t" count
 } else if(c=="\""){string();print path "\tstring\t" value}
 else if(substr(text,pos,4)=="true"){pos+=4;print path "\tboolean\ttrue"}
 else if(substr(text,pos,5)=="false"){pos+=5;print path "\tboolean\tfalse"}
 else if(substr(text,pos,4)=="null"){pos+=4;print path "\tnull\tnull"}
 else {
  start=pos
  if(c=="-")pos++
  c=substr(text,pos,1)
  if(c=="0")pos++
  else if(c~/^[1-9]$/){pos++;while(substr(text,pos,1)~/^[0-9]$/)pos++}
  else bad()
  if(substr(text,pos,1)=="."){pos++;if(substr(text,pos,1)!~/^[0-9]$/)bad();while(substr(text,pos,1)~/^[0-9]$/)pos++}
  c=substr(text,pos,1)
  if(c=="e"||c=="E"){pos++;c=substr(text,pos,1);if(c=="+"||c=="-")pos++;if(substr(text,pos,1)!~/^[0-9]$/)bad();while(substr(text,pos,1)~/^[0-9]$/)pos++}
  print path "\tnumber\t" substr(text,start,pos-start)
 }
}
BEGIN { for(i=0;i<256;i++)ord[sprintf("%c",i)]=i; pos=1 }
{if(NR>1)text=text "\n";text=text $0}
END {
 if(failed)exit 1
 if(action=="quote") {
  printf "\"";for(i=1;i<=length(text);i++){c=substr(text,i,1);n=ord[c];if(c=="\""||c=="\\")printf "\\%s",c;else if(n<32)printf "\\u%04x",n;else printf "%s",c}printf "\"";exit
 }
 node("$",0);ws();if(pos<=length(text))bad()
}
JSON_AWK_END
)
json_parse() { awk "$JSON_AWK"; }
json_quote() { printf '%s' "$1" | awk -v action=quote "$JSON_AWK"; }
hex_key() { printf '%s' "$1" | od -An -tx1 | tr -d ' \n'; }
jget() { printf '%s\n' "$JSON_DATA" | awk -F '\t' -v key="$1" -v type="$2" '$1==key && $2==type{print $3;found=1;exit} END{if(!found)exit 1}'; }
jstring() {
  local encoded rest pair decoded=''
  encoded=$(jget "$1" string) || return 1
  rest=$encoded
  while [[ -n $rest ]]; do
    pair=${rest:0:2}; rest=${rest:2}
    case "$pair" in 0[0-9a-f]|1[0-9a-f]|7f) return 1;; esac
    printf -v decoded '%s\\x%s' "$decoded" "$pair"
  done
  printf '%b' "$decoded"
}
jfield() { jstring "\$/$(hex_key "$1")"; }
repo_valid() { [[ $1 =~ ^[a-zA-Z0-9_.-]+/[a-zA-Z0-9_.-]+$ && ${1%/*} != . && ${1%/*} != .. && ${1#*/} != . && ${1#*/} != .. ]]; }
normal_tag() {
  [[ -n $1 ]] || { printf ''; return; }
  [[ $1 =~ $VERSION_RE ]] || die "invalid $2: $1; expected 0.0.N or v0.0.N"
  printf 'v%s' "${1#v}"
}
no_links() {
  local path=$1
  while [[ -n $path && $path != / ]]; do [[ ! -L $path ]] || return 1; path=${path%/*}; done
}
normalize() {
  local part result='' old_ifs=$IFS parts
  [[ $1 == /* && $1 != *..* && $1 != *:* && ! $1 =~ [[:cntrl:]] ]] || return 1
  IFS=/ read -r -a parts <<< "$1"; IFS=$old_ifs
  for part in "${parts[@]}"; do [[ -z $part || $part == . ]] || result="$result/$part"; done
  [[ -n $result ]] || return 1
  no_links "$result" || return 1
  [[ ! -e $result || -d $result ]] || return 1
  NORMAL=$result
}
unsafe() { die "unsafe installation path: $1" 'choose absolute, non-overlapping envswitch paths without symlinks or reparse points'; }
unmanaged() { die "refusing to replace an unmanaged path: $1" 'preserve user files and reinstall using the README instructions with explicit install paths'; }
path_ok() { no_links "$1" || unsafe "$1"; }
quote() { local value=${1//\'/\'\\\'\'}; printf "'%s'" "$value"; }
escaped() { local value=$1; value=${value//\\/\\\\}; value=${value//\"/\\\"}; value=${value//\$/\\\$}; value=${value//\`/\\\`}; printf '%s' "$value"; }
launcher() {
  printf '#!/usr/bin/env bash\nset -euo pipefail\n'
  if [[ $1 == source ]]; then
    cat <<'SOURCE'
if command -v bun >/dev/null 2>&1; then
  BUN_BIN="$(command -v bun)"
elif [[ -x "${HOME}/.bun/bin/bun" ]]; then
  BUN_BIN="${HOME}/.bun/bin/bun"
else
  printf 'envswitch: bun is not installed or not on PATH\n' >&2
  exit 127
fi
SOURCE
    # shellcheck disable=SC2016
    printf 'exec "$BUN_BIN" '
  else printf 'exec '; fi
  if [[ ${3:-} == legacy ]]; then printf '"%s"' "$2"; else quote "$2"; fi; printf ' "$@"\n'
}
payload_for() {
  if [[ $2 == source ]]; then printf '%s/current/src/cli.ts' "$1"; else printf '%s/current/bin/envswitch_%s_%s' "$1" "${3#v}" "$4"; fi
}
read_receipt() {
  P_PRESENT=0; P_TAG='' P_KIND='' P_TARGET='' P_PAYLOAD='' P_ROOT='' P_DEST='' P_LAUNCH='' P_REPO=''
  [[ -e $APP/install.json ]] || return 0
  if ! valid_receipt; then die "invalid envswitch install receipt: $APP/install.json" 'reinstall envswitch with the install script from the README after checking the install paths'; fi
  P_PRESENT=1
}
valid_receipt() {
  JSON_DATA=$(json_parse < "$APP/install.json") || return 1
  [[ $(jget '$' object) =~ ^[0-9]+$ && $(jget "\$/$(hex_key schemaVersion)" number) == 1 ]] || return 1
  P_KIND=$(jfield kind) && P_TAG=$(jfield tag) && P_REPO=$(jfield repo) && P_ROOT=$(jfield appRoot) && P_DEST=$(jfield installDir) && P_LAUNCH=$(jfield launcherPath) && P_PAYLOAD=$(jfield payloadPath) || return 1
  [[ $P_KIND == source || $P_KIND == native ]] && [[ $P_TAG =~ ^v0\.0\.(0|[1-9][0-9]*)$ ]] && repo_valid "$P_REPO" || return 1
  normalize "$P_ROOT" && [[ $NORMAL == "$P_ROOT" && $P_ROOT == "$ROOT" && $P_ROOT != "$HOME" && $P_ROOT != "${USERPROFILE:-}" ]] || return 1
  normalize "$P_DEST" && [[ $NORMAL == "$P_DEST" && $P_DEST != "$P_ROOT" && $P_DEST != "$P_ROOT/current" && $P_DEST != "$P_ROOT/current/"* && $P_DEST != */.install-* ]] || return 1
  [[ $P_LAUNCH == "$P_DEST/envswitch" ]] || return 1
  if [[ $P_KIND == source ]]; then [[ $(jget "\$/$(hex_key target)" null) == null ]] || return 1
  else
    P_TARGET=$(jfield target) || return 1
    case "$P_TARGET" in aarch64-apple-darwin|x86_64-apple-darwin|x86_64-unknown-linux-musl|aarch64-unknown-linux-musl) ;; *) return 1;; esac
  fi
  [[ $P_PAYLOAD == "$(payload_for "$P_ROOT" "$P_KIND" "$P_TAG" "$P_TARGET")" ]] && no_links "$P_LAUNCH" && no_links "$P_PAYLOAD"
}
recognize_launcher() {
  [[ ! -e $LAUNCH ]] && return 0
  [[ -f $LAUNCH ]] || return 1
  local content expected version target
  content=$(cat "$LAUNCH")
  if [[ $P_PRESENT == 1 ]]; then expected=$(launcher "$P_KIND" "$P_PAYLOAD"); [[ $content != "$expected" ]] || return 0; fi
  expected=$(launcher source "$APP/src/cli.ts"); [[ $content != "$expected" ]] || return 0
  expected=$(launcher source "$APP/src/cli.ts" legacy)
  [[ $(printf '%s\n' "$content" | sed '/^$/d') != "$expected" ]] || return 0
  if [[ $content =~ envswitch_(0\.0\.(0|[1-9][0-9]*))_ ]]; then
    version="v${BASH_REMATCH[1]}"
    for target in "${TARGETS[@]}"; do
      expected=$(launcher native "$(payload_for "$ROOT" native "$version" "$target")")
      [[ $content != "$expected" ]] || return 0
    done
  fi
  return 1
}
ensure_dir() {
  [[ -d $1 ]] && { path_ok "$1"; return; }
  [[ ! -e $1 && ! -L $1 ]] || unsafe "$1"
  local parent=${1%/*}
  ensure_dir "${parent:-/}"
  mkdir "$1"; CREATED[${#CREATED[@]}]=$1
}
# Capture bytes, bound verification/gh processes, and reap them before rollback.
bounded() {
  local limit=$1 out=$2 err=$3 status=0 child timer
  shift 3
  "$@" > "$out" 2> "$err" & child=$!; ACTIVE_CHILD=$child
  (
    sleeper=''
    trap '[[ -z $sleeper ]] || { kill "$sleeper" 2>/dev/null || :; wait "$sleeper" 2>/dev/null || :; }; exit 0' TERM INT
    sleep "$limit" & sleeper=$!; wait "$sleeper" || exit 0; sleeper=''
    kill -TERM "$child" 2>/dev/null || exit 0; sleep 1; kill -KILL "$child" 2>/dev/null || :
  ) >/dev/null 2>&1 & timer=$!; WATCHDOG=$timer
  wait "$child" || status=$?
  kill "$timer" 2>/dev/null || :; wait "$timer" 2>/dev/null || :
  ACTIVE_CHILD='' WATCHDOG=''
  return "$status"
}
cleanup() {
  local status=$? i path backup failed=0
  trap - ERR
  if [[ -n $ACTIVE_CHILD ]]; then kill -TERM "$ACTIVE_CHILD" 2>/dev/null || :; wait "$ACTIVE_CHILD" 2>/dev/null || :; fi
  [[ -z $WATCHDOG ]] || { kill "$WATCHDOG" 2>/dev/null || :; wait "$WATCHDOG" 2>/dev/null || :; }
  if [[ $COMMITTED == 0 ]]; then
    if [[ $PROFILE_WRITTEN == 1 ]]; then
      if [[ $PROFILE_EXISTED == 1 ]]; then cat "$PROFILE_BACKUP" > "$PROFILE_PATH" || failed=1
      elif [[ ! -L $PROFILE_PATH ]]; then rm -f "$PROFILE_PATH" || failed=1; fi
    fi
    i=${#J_PATH[@]}
    while [[ $i -gt 0 ]]; do
      i=$((i-1)); [[ ${J_CHANGED[i]} == 1 ]] || continue
      path=${J_PATH[i]}; backup=${J_BACKUP[i]}
      if [[ -d $path ]]; then rm -rf "$path" || failed=1; fi
      if [[ -n $backup ]]; then mv -f "$backup" "$path" || failed=1
      else rm -f "$path" || failed=1; fi
    done
    if [[ $failed == 0 && -n $NEW_PAYLOAD ]]; then rm -f "$NEW_PAYLOAD" || info "deferred cleanup: $NEW_PAYLOAD"; fi
  fi
  if [[ $LOCKED == 1 && ! -L $LOCK && -f $LOCK && $(cat "$LOCK") == "$TOKEN $$" ]]; then rm -f "$LOCK" || failed=1; fi
  LOCKED=0
  for path in "${TEMPS[@]:-}"; do [[ -z $path ]] || rm -rf "$path" || failed=1; done
  if [[ $failed == 1 ]]; then printf 'error: rollback incomplete; preserve %s and reinstall using the README instructions\n' "$STAGE" >&2
  elif [[ -n $STAGE && -d $STAGE ]]; then rm -rf "$STAGE" || info "deferred cleanup: $STAGE"; fi
  STAGE=''
  i=${#CREATED[@]}; while [[ $i -gt 0 ]]; do i=$((i-1)); rmdir "${CREATED[i]}" 2>/dev/null || :; done
  return "$status"
}
replace_managed() {
  local path=$1 source=$2 mode=${3:-600} i backup='' tmp moved=0
  path_ok "$path"; ensure_dir "${path%/*}"
  i=${#J_PATH[@]}
  if [[ -e $path ]]; then backup="$STAGE/backup-$i"; cp -pR "$path" "$backup"; fi
  if [[ -d $source ]]; then
    tmp=$(mktemp -d "${path%/*}/.install-XXXXXXXX"); rmdir "$tmp"; cp -pR "$source" "$tmp"
  else tmp=$(mktemp "${path%/*}/.install-XXXXXXXX"); cat "$source" > "$tmp"; chmod "$mode" "$tmp"; fi
  TEMPS[${#TEMPS[@]}]=$tmp
  J_PATH[i]=$path; J_BACKUP[i]=$backup; J_CHANGED[i]=0
  if [[ -d $path ]]; then mv "$path" "$STAGE/moved-$i"; J_CHANGED[i]=1; moved=1; fi
  path_ok "$path"
  if mv -f "$tmp" "$path"; then J_CHANGED[i]=1
  else [[ $moved == 0 ]] || J_CHANGED[i]=1; die 'envswitch installation failed: could not replace a managed path'; fi
}
boundary() { if [[ $TEST == 1 && ${ENVSWITCH_INSTALL_TEST_FAIL_AT:-} == "$1" ]]; then die "injected installer test failure: $1"; fi; }
shell_profile_path() {
  case "${SHELL##*/}" in zsh) printf '%s/.zshrc\n' "$HOME";; bash) printf '%s/.bashrc\n' "$HOME";; *) printf '%s/.profile\n' "$HOME";; esac
}
ensure_profile_path() {
  local install=$1 profile=$2 work old last block msg bytes
  [[ ! -L $profile || -e $profile ]] || return 0
  work=$(mktemp -d "${TMPDIR:-/tmp}/envswitch-profile-XXXXXXXX"); TEMPS[${#TEMPS[@]}]=$work
  old="$work/old"; block="$work/block"
  [[ ! -e $profile || -f $profile ]] || unmanaged "$profile"
  if [[ -f $profile ]]; then cat "$profile" > "$old"; else : > "$old"; fi
  # shellcheck disable=SC2016
  printf '%s\nexport PATH="%s:$PATH"\n%s\n' "$BEGIN" "$(escaped "$install")" "$END" > "$block"
  last=$(tail -c 1 "$old" | od -An -tu1 | tr -d ' \n')
  if ! awk -v begin="$BEGIN" -v end="$END" -v block="$block" -v finalnl="$last" '
    {lines[NR]=$0;clean[NR]=$0;crlf[NR]=sub(/\r$/,"",clean[NR]);if(index(clean[NR],begin)){a=NR;as++}if(index(clean[NR],end)){b=NR;bs++}}
    END{
      if(as||bs){if(as!=1||bs!=1||b!=a+2||clean[a]!=begin||clean[b]!=end||clean[a+1]!~/^export PATH=".*:\$PATH"$/)exit 1}
      eol=(crlf[a?a:1]?"\r\n":"\n")
      for(i=1;i<=NR;i++){
        if(i==a){while((getline line < block)>0)printf "%s%s",line,eol;close(block);i=b}
        else printf "%s%s",lines[i],(i<NR||finalnl=="10"?"\n":"")
      }
      if(!as){printf "%s",(NR&&finalnl!="10"?eol eol:eol);while((getline line < block)>0)printf "%s%s",line,eol;close(block)}
    }' "$old" > "$work/new"; then die "invalid envswitch PATH block in $profile; repair the block and rerun the installer"; fi
  if cmp -s "$old" "$work/new"; then info "PATH block already exists in $profile"; return; fi
  if grep -Fq "$BEGIN" "$old"; then msg="updated envswitch PATH block in $profile"; else msg="added $install to PATH in $profile"; fi
  # Follow a user-managed profile symlink deliberately, retaining the link and target inode.
  if [[ -n $STAGE ]]; then
    PROFILE_PATH=$profile; PROFILE_BACKUP="$STAGE/profile-backup"; cp -p "$old" "$PROFILE_BACKUP"
    [[ ! -e $profile && ! -L $profile ]] || PROFILE_EXISTED=1
    PROFILE_WRITTEN=1
  fi
  mkdir -p "${profile%/*}"
  if grep -Fq "$BEGIN" "$old"; then cat "$work/new" > "$profile"
  else bytes=$(wc -c < "$old"); tail -c +$((bytes+1)) "$work/new" >> "$profile"; fi
  info "$msg"
}
asset_url_valid() {
  local url=$1 id=$2 rest repo lower wanted
  [[ $id =~ ^[1-9][0-9]*$ && $url == "$API_BASE/repos/"*"/releases/assets/$id" ]] || return 1
  rest=${url#"$API_BASE/repos/"}; repo=${rest%"/releases/assets/$id"}
  lower=$(printf '%s' "$repo" | tr '[:upper:]' '[:lower:]'); wanted=$(printf '%s' "$REPO" | tr '[:upper:]' '[:lower:]')
  [[ $lower == "$wanted" ]]
}
redirect_valid() {
  case "$1" in
    https://api.github.com/*|https://release-assets.githubusercontent.com/*|https://objects.githubusercontent.com/*|https://github-releases.githubusercontent.com/*) return 0;;
    http://127.0.0.1:*/*|http://localhost:*/*|http://\[::1\]:*/*) [[ $TEST == 1 ]];;
    *) return 1;;
  esac
}
retry_delay() {
  local delay=$1 retry_after=$2 until_epoch now
  if [[ $retry_after =~ ^[0-9]+$ ]]; then
    delay=$(awk -v requested="$retry_after" -v minimum="$delay" 'BEGIN{n=requested+0;if(n>60)n=60;if(n<minimum)n=minimum;printf "%.0f",n}')
  elif [[ -n $retry_after ]]; then
    until_epoch=$(date -d "$retry_after" +%s 2>/dev/null || TZ=UTC0 date -D '%a, %d %b %Y %H:%M:%S GMT' -d "$retry_after" +%s 2>/dev/null || date -j -f '%a, %d %b %Y %H:%M:%S %Z' "$retry_after" +%s 2>/dev/null || printf 0)
    now=$(date +%s); [[ $until_epoch -le $((now+delay)) ]] || delay=$((until_epoch-now))
  fi
  [[ $delay -le 60 ]] || delay=60
  printf '%s' "$delay"
}
http_get() {
  local original=$1 out=$2 accept=$3 transfer=${4:-60} attempt=0 url code next auth token status delay retry_after redirect
  local proto='=https'
  [[ $TEST == 0 ]] || proto='=http,https'
  need curl
  while [[ $attempt -lt 3 ]]; do
    attempt=$((attempt+1)); url=$original; redirect=0; status=0
    while [[ $redirect -lt 10 ]]; do
      auth=''
      if [[ -n ${GITHUB_TOKEN:-} && $url == "$API_BASE/"* ]]; then
        token=${GITHUB_TOKEN//\\/\\\\}; token=${token//\"/\\\"}
        [[ ! $token =~ [[:cntrl:]] ]] || setting GITHUB_TOKEN
        auth="header = \"Authorization: Bearer $token\""
      fi
      # No -L: authenticate only the API origin, then validate each redirect independently.
      if printf '%s\n' "$auth" | curl -q --config - --proto "$proto" --proto-redir '=https' --connect-timeout 15 --max-time "$transfer" --speed-limit 1 --speed-time 60 -sS -H "Accept: $accept" -D "$STAGE/headers" -o "$out" -w '%{http_code}\n%{redirect_url}\n' "$url" > "$STAGE/http-status" 2> "$STAGE/http-error"; then status=0; else status=$?; fi
      code=$(sed -n '1p' "$STAGE/http-status"); next=$(sed -n '2p' "$STAGE/http-status")
      [[ $status == 0 ]] || break
      case "$code" in
        200) return 0;;
        301|302|303|307|308) redirect_valid "$next" || return 1; url=$next; redirect=$((redirect+1)); continue;;
        408|429|5[0-9][0-9]) break;;
        *) return 1;;
      esac
    done
    [[ $redirect -lt 10 && $attempt -lt 3 ]] || return 1
    retry_after=$(awk 'tolower($1)=="retry-after:"{$1="";sub(/^ /,"");sub(/\r$/,"");print;exit}' "$STAGE/headers")
    delay=$(retry_delay "$((1 << (attempt-1)))" "$retry_after")
    sleep "$delay"
  done
  return 1
}
select_release() {
  local endpoint data_ok=0 id count index path name url assetid natives=0 native
  endpoint="repos/$REPO/releases/latest"; [[ -z $PIN ]] || endpoint="repos/$REPO/releases/tags/$PIN"
  if [[ -n $GH ]]; then bounded 60 "$STAGE/release.json" "$STAGE/gh-error" "$GH" api "$endpoint" && data_ok=1 || :; fi
  if [[ $data_ok == 0 ]]; then
    if ! http_get "$API_BASE/$endpoint" "$STAGE/release.json" application/vnd.github+json; then
      [[ -z $PIN ]] || die "GitHub Release not found or unavailable: $REPO@$PIN"
      die "no GitHub Release found for $REPO, or access is denied"
    fi
  fi
  JSON_DATA=$(json_parse < "$STAGE/release.json") || die "$BAD_RELEASE"
  TAG=$(jfield tag_name) || die "$BAD_RELEASE"; id=$(jget "\$/$(hex_key id)" number) || die "$BAD_RELEASE"
  [[ $(jget '$' object) =~ ^[0-9]+$ && $TAG =~ ^v0\.0\.(0|[1-9][0-9]*)$ && $id =~ ^[1-9][0-9]*$ && ( -z $PIN || $PIN == "$TAG" ) && $(jget "\$/$(hex_key draft)" boolean) == false && $(jget "\$/$(hex_key prerelease)" boolean) == false ]] || die "$BAD_RELEASE"
  count=$(jget "\$/$(hex_key assets)" array) || die "$BAD_RELEASE"
  ASSET_NAMES=(); ASSET_URLS=(); index=0
  while [[ $index -lt $count ]]; do
    path="\$/$(hex_key assets)/$index"
    [[ $(jget "$path" object) =~ ^[0-9]+$ ]] || die "$BAD_RELEASE"
    if ! { name=$(jstring "$path/$(hex_key name)") && url=$(jstring "$path/$(hex_key url)") && assetid=$(jget "$path/$(hex_key id)" number); }; then die "$BAD_RELEASE"; fi
    asset_url_valid "$url" "$assetid" || die "$BAD_RELEASE"
    ASSET_NAMES[index]=$name; ASSET_URLS[index]=$url; index=$((index+1))
  done
  VER=${TAG#v}; SOURCE="envswitch_${VER}_source.tar.gz"; NATIVE_NAMES=()
  for native in "${TARGETS[@]}"; do
    name="envswitch_${VER}_${native}.tar.gz"; [[ $native != *-msvc ]] || name="envswitch_${VER}_${native}.zip"
    NATIVE_NAMES[${#NATIVE_NAMES[@]}]=$name
    asset_count "$name"; [[ $COUNT == 0 ]] || natives=$((natives+1))
    if [[ $native == "$TARGET" ]]; then ARCHIVE=$name; fi
  done
  KIND=source; [[ $natives == 0 ]] || KIND=native
  if [[ $KIND == source ]]; then ARCHIVE=$SOURCE; REQUIRED=("$SOURCE"); else REQUIRED=("${NATIVE_NAMES[@]}"); asset_count "$SOURCE"; [[ $COUNT == 0 ]] || die "$BAD_RELEASE"; fi
  for name in "${REQUIRED[@]}"; do asset_count "$name"; [[ $COUNT == 1 ]] || die "$BAD_RELEASE"; done
  asset_count SHA256SUMS; [[ $COUNT != 0 ]] || die "release $TAG is missing SHA256SUMS; refusing unverified installation"
  for name in SHA256SUMS install.sh install.ps1; do asset_count "$name"; [[ $COUNT == 1 ]] || die "$BAD_RELEASE"; done
  info "resolved envswitch $TAG ($KIND)"; [[ $KIND != native ]] || info "selected target: $TARGET"
}
asset_count() {
  local i=0; COUNT=0; ASSET_URL=''
  while [[ $i -lt ${#ASSET_NAMES[@]} ]]; do
    if [[ ${ASSET_NAMES[i]} == "$1" ]]; then COUNT=$((COUNT+1)); ASSET_URL=${ASSET_URLS[i]}; fi
    i=$((i+1))
  done
}
download() {
  local name=$1 good=0
  if [[ -n $GH ]]; then
    info "downloading $name with GitHub CLI"
    bounded 900 "$STAGE/gh-out" "$STAGE/gh-error" "$GH" release download "$TAG" --repo "$REPO" --pattern "$name" --dir "$STAGE" && good=1 || :
    [[ $good == 0 || ! -f $STAGE/$name ]] || return 0
    rm -f "$STAGE/$name"; info "GitHub CLI download did not find $name; trying direct download"
  else info "downloading $name"; fi
  asset_count "$name"
  http_get "$ASSET_URL" "$STAGE/$name" application/octet-stream 900 || die "release asset not found or unavailable: $name"
}
manifest() {
  [[ $(tail -c 1 "$STAGE/SHA256SUMS" | od -An -tu1 | tr -d ' \n') == 10 ]] || die "invalid SHA256SUMS for release $TAG"
  awk '
    {if(length($0)<67||substr($0,65,2)!="  ")exit 1;h=substr($0,1,64);n=substr($0,67);if(h~/[^0-9a-f]/||length(h)!=64||n~/[\/\\[:cntrl:] ]/||n==""||n=="."||n==".."||n=="SHA256SUMS"||seen[n]++)exit 1}
  ' "$STAGE/SHA256SUMS" || die "invalid SHA256SUMS for release $TAG"
  local name
  for name in "${REQUIRED[@]}" install.sh install.ps1; do
    awk -v name="$name" 'substr($0,67)==name{found=1}END{exit !found}' "$STAGE/SHA256SUMS" || die "invalid SHA256SUMS for release $TAG"
  done
}
verify_hash() {
  local expected actual
  info "verifying SHA-256 for $2"
  expected=$(awk -v name="$2" 'substr($0,67)==name{print substr($0,1,64)}' "$STAGE/SHA256SUMS")
  if [[ $HASH == sha256sum ]]; then actual=$(sha256sum "$1"); else actual=$(shasum -a 256 "$1"); fi
  actual=${actual%% *}; actual=$(printf '%s' "$actual" | tr '[:upper:]' '[:lower:]')
  [[ $actual == "$expected" ]] || die "SHA-256 mismatch for $2"
}
archive_valid() {
  local name type listing normalized top rest entry count=0
  tar -tzf "$STAGE/$ARCHIVE" > "$STAGE/names" 2> "$STAGE/tar-error" || return 1
  tar -tvzf "$STAGE/$ARCHIVE" > "$STAGE/verbose" 2> "$STAGE/tar-error" || return 1
  cut -c1 "$STAGE/verbose" > "$STAGE/types"
  [[ $(wc -l < "$STAGE/types") == "$(wc -l < "$STAGE/names")" ]] || return 1
  : > "$STAGE/normalized"; : > "$STAGE/expected-files"
  exec 3< "$STAGE/types"
  exec 4< "$STAGE/verbose"
  while IFS='' read -r name; do
    IFS='' read -r type <&3 || return 1
    IFS= read -r listing <&4 || return 1
    [[ $name =~ ^[A-Za-z0-9._/-]+$ ]] || return 1
    [[ $type == - || $type == d ]] || return 1
    [[ $type != - || ( $listing != *" -> "* && $listing != *" link to "* ) ]] || return 1
    normalized=${name#./}
    if [[ $type == d ]]; then normalized=${normalized%/}; fi
    [[ -n $normalized && $normalized != /* && $normalized != *\\* && $normalized != *:* && ! $normalized =~ [[:cntrl:]] && $normalized != */ && $normalized != *//* ]] || return 1
    case "/$normalized/" in */./*|*/../*) return 1;; esac
    if grep -Fxq "$normalized" "$STAGE/normalized"; then return 1; fi
    printf '%s\n' "$normalized" >> "$STAGE/normalized"
    if [[ $KIND == native ]]; then
      if [[ $type == d ]]; then [[ $normalized == scripts ]] || return 1
      else case "$normalized" in envswitch|README.md|scripts/install.sh|scripts/install.ps1) ;; *) return 1;; esac; fi
    else
      [[ $normalized == envswitch || $normalized == envswitch/* ]] || return 1
      rest=${normalized#envswitch/}; top=${rest%%/*}
      if [[ $normalized != envswitch ]]; then
        case "$top" in src|scripts) ;; package.json|bun.lock|README.md|tsconfig.json) [[ $rest == "$top" && $type == - ]] || return 1;; *) return 1;; esac
      else [[ $type == d ]] || return 1; fi
      case "/$normalized/" in */.git/*|*/node_modules/*|*/.env/*|*/.env.*/*) return 1;; esac
    fi
    [[ $type != - ]] || printf '%s\n' "$normalized" >> "$STAGE/expected-files"
    count=$((count+1))
  done < "$STAGE/names"
  exec 3<&-
  exec 4<&-
  [[ $count -gt 0 ]] || return 1
  if [[ $KIND == native ]]; then
    for entry in envswitch README.md scripts/install.sh scripts/install.ps1; do grep -Fxq "$entry" "$STAGE/expected-files" || return 1; done
  else
    for entry in package.json bun.lock src/cli.ts scripts/install.sh scripts/install.ps1; do grep -Fxq "envswitch/$entry" "$STAGE/expected-files" || return 1; done
  fi
}
extract_archive() {
  archive_valid || die "unsafe or invalid archive: $ARCHIVE"
  TREE=$(mktemp -d "$STAGE/extracted-XXXXXXXX")
  tar --no-same-owner --no-same-permissions -xzf "$STAGE/$ARCHIVE" -C "$TREE" 2> "$STAGE/tar-error" || die "unsafe or invalid archive: $ARCHIVE"
  find "$TREE" ! -type f ! -type d -print > "$STAGE/unsafe-entries" 2> "$STAGE/find-error" || die "unsafe or invalid archive: $ARCHIVE"
  find "$TREE" -type f -links +1 -print > "$STAGE/linked-files" 2> "$STAGE/find-error" || die "unsafe or invalid archive: $ARCHIVE"
  [[ ! -s $STAGE/unsafe-entries && ! -s $STAGE/linked-files ]] || die "unsafe or invalid archive: $ARCHIVE"
  : > "$STAGE/actual-files"
  while IFS='' read -r entry; do printf '%s\n' "${entry#"$TREE/"}" >> "$STAGE/actual-files"; chmod 600 "$entry"; done < <(find "$TREE" -type f -print)
  sort "$STAGE/actual-files" > "$STAGE/actual-sorted"; sort "$STAGE/expected-files" > "$STAGE/expected-sorted"
  cmp -s "$STAGE/actual-sorted" "$STAGE/expected-sorted" || die "unsafe or invalid archive: $ARCHIVE"
  [[ $KIND != source ]] || TREE="$TREE/envswitch"
}
check_payload() {
  local file=$1 kind=$2 staged=${3:-0}
  if [[ $kind == native ]]; then
    printf 'envswitch %s %s\n' "$VER" "$TARGET" > "$STAGE/expected-version"
    if ! bounded 15 "$STAGE/version-out" "$STAGE/version-err" "$file" --version || [[ -s $STAGE/version-err ]] || ! cmp -s "$STAGE/expected-version" "$STAGE/version-out"; then
      [[ $staged == 0 ]] || die "native payload verification failed; expected envswitch $VER $TARGET"
      return 1
    fi
  fi
  bounded 15 "$STAGE/help-out" "$STAGE/help-err" "$file" --help
}
source_shape() {
  local dir=$1 required=${2:-1} entry rel first
  if [[ -e $dir/package.json ]]; then
    JSON_DATA=$(json_parse < "$dir/package.json") || return 1
    [[ $(jfield name) == envswitch ]] || return 1
  elif [[ $required == 1 ]]; then return 1; fi
  if [[ $required == 1 ]]; then [[ -f $dir/src/cli.ts && -f $dir/bun.lock ]] || return 1; fi
  while IFS='' read -r entry; do
    rel=${entry#"$dir/"}; first=${rel%%/*}
    [[ ! -L $entry && ( -f $entry || -d $entry ) && ! $rel =~ [[:cntrl:]] ]] || return 1
    case "$first" in src|scripts|node_modules|package.json|bun.lock|README.md|tsconfig.json) ;; *) return 1;; esac
    case "/$rel/" in */.git/*|*/.env/*|*/.env.*/*) return 1;; esac
  done < <(find "$dir" -mindepth 1 -print)
}
ensure_bun() {
  BUN_BIN=$(command -v bun || :)
  [[ -n $BUN_BIN || ! -x $HOME/.bun/bin/bun ]] || BUN_BIN="$HOME/.bun/bin/bun"
  if [[ -z $BUN_BIN ]]; then
    command -v curl >/dev/null 2>&1 || die 'curl is required to install Bun automatically' 'install Bun manually from https://bun.com/docs/installation, then rerun this installer'
    info 'Bun is not installed; installing Bun'
    if ! curl -q --proto '=https' --proto-redir '=https' -fsSL https://bun.com/install -o "$STAGE/bun-bootstrap"; then die 'Bun installation finished, but bun was not found on PATH'; fi
    if ! bash "$STAGE/bun-bootstrap" > "$STAGE/bun-out" 2> "$STAGE/bun-err"; then cat "$STAGE/bun-out"; cat "$STAGE/bun-err" >&2; die 'Bun installation finished, but bun was not found on PATH'; fi
    export BUN_INSTALL="${BUN_INSTALL:-$HOME/.bun}"; export PATH="$BUN_INSTALL/bin:$PATH"; BUN_BIN=$(command -v bun || :)
    [[ -n $BUN_BIN ]] || die 'Bun installation finished, but bun was not found on PATH'
  fi
  info "using Bun at $BUN_BIN"
  bounded 15 "$STAGE/bun-out" "$STAGE/bun-err" "$BUN_BIN" --version || { cat "$STAGE/bun-out"; cat "$STAGE/bun-err" >&2; die 'Bun verification failed'; }
  info 'installing production dependencies'
  if ! (cd "$TREE" && "$BUN_BIN" install --production --frozen-lockfile) > "$STAGE/bun-out" 2> "$STAGE/bun-err"; then cat "$STAGE/bun-out"; cat "$STAGE/bun-err" >&2; die 'bun install failed'; fi
}
write_receipt() {
  printf '{"schemaVersion":1,"kind":%s,"repo":%s,"appRoot":%s,"installDir":%s,"launcherPath":%s,"target":%s,"tag":%s,"payloadPath":%s}\n' \
    "$(json_quote "$KIND")" "$(json_quote "$REPO")" "$(json_quote "$ROOT")" "$(json_quote "$DEST")" "$(json_quote "$LAUNCH")" \
    "$([[ $KIND == native ]] && json_quote "$TARGET" || printf null)" "$(json_quote "$TAG")" "$(json_quote "$PAYLOAD")" > "$STAGE/receipt"
  printf '%s\n' "$TAG" > "$STAGE/marker"
}
main() {
  local mode=${ENVSWITCH_INSTALL_TEST_MODE:-} old=${ENVS_SWITCH_INSTALL_TEST_MODE:-} os arch tool name entry existing healthy=0 oldtag='' outcome profile
  [[ -n ${HOME:-} ]] || die 'HOME is not set; set HOME before running the installer'
  SHELL=${SHELL:-}
  [[ -z $old || -z $mode ]] || setting ENVSWITCH_INSTALL_TEST_MODE
  case "$old" in ''|ensure-profile-path|shell-profile-path) ;; *) setting ENVS_SWITCH_INSTALL_TEST_MODE;; esac
  mode=${mode:-$old}
  case "$mode" in ''|install|ensure-profile-path|shell-profile-path) ;; *) setting ENVSWITCH_INSTALL_TEST_MODE;; esac
  for tool in bash awk cat od tr sed mktemp find tar uname cp mv rm rmdir mkdir chmod cmp sort grep cut tail date sleep wc; do need "$tool"; done
  if [[ $mode == shell-profile-path ]]; then [[ $# == 0 ]] || die "unexpected installer argument: $1"; shell_profile_path; return; fi
  if [[ $mode == ensure-profile-path ]]; then [[ $# == 2 ]] || setting ENVSWITCH_INSTALL_TEST_MODE; ensure_profile_path "$1" "$2"; return; fi
  [[ $# == 0 ]] || die "unexpected installer argument: $1"
  TEST=0; API_BASE=https://api.github.com
  if [[ $mode == install ]]; then
    TEST=1; API_BASE=${ENVSWITCH_INSTALL_TEST_API_BASE_URL:-}
    [[ $API_BASE =~ ^http://(127\.0\.0\.1|localhost|\[::1\])(:[0-9]+)?$ ]] || setting ENVSWITCH_INSTALL_TEST_API_BASE_URL
    case "${ENVSWITCH_INSTALL_TEST_FAIL_AT:-}" in ''|after-payload|after-launcher|after-metadata) ;; *) setting ENVSWITCH_INSTALL_TEST_FAIL_AT;; esac
  fi
  REPO=${REPO:-rnldsalili/envswitch}; BAD_RELEASE="invalid envswitch release metadata for $REPO"
  repo_valid "$REPO" || die "invalid repository format: $REPO" 'expected owner/repo'
  PIN=$(normal_tag "${ENVSWITCH_VERSION:-}" Version); CURRENT=$(normal_tag "${ENVSWITCH_CURRENT_VERSION:-}" CurrentVersion)
  ATTEST=${ENVSWITCH_VERIFY_ATTESTATIONS:-0}; case "$ATTEST" in ''|0|1) ;; *) setting ENVSWITCH_VERIFY_ATTESTATIONS;; esac
  ROOT=${APP_ROOT:-${XDG_DATA_HOME:-$HOME/.local/share}/envswitch}; DEST=${INSTALL_DIR:-$HOME/.local/bin}
  for name in APP_ROOT INSTALL_DIR; do
    entry=$ROOT; [[ $name != INSTALL_DIR ]] || entry=$DEST
    [[ $entry == /* && $entry != *..* ]] || die "invalid $name: $entry" "$name must be an absolute path without '..'"
    normalize "$entry" || unsafe "$entry"
    if [[ $name == APP_ROOT ]]; then ROOT=$NORMAL; else DEST=$NORMAL; fi
  done
  APP="$ROOT/current"; LAUNCH="$DEST/envswitch"; LOCK="$ROOT/.install.lock"
  [[ $ROOT != "$HOME" && $ROOT != "${USERPROFILE:-}" && $DEST != "$ROOT" && $DEST != "$APP" && $DEST != "$APP/"* && $ROOT != "$LAUNCH" && $ROOT != "$LAUNCH/"* && $ROOT != */.install-* && $DEST != */.install-* ]] || unsafe "$ROOT"
  for entry in "$APP" "$LAUNCH" "$APP/bin" "$APP/scripts" "$APP/scripts/install.sh" "$APP/scripts/install.ps1" "$APP/install.json" "$APP/.version"; do path_ok "$entry"; done
  [[ ! -e $APP || -d $APP ]] || unsafe "$APP"
  for entry in "$ROOT/.git" "$APP/.git"; do [[ ! -e $entry && ! -L $entry ]] || unmanaged "$entry"; done
  read_receipt; recognize_launcher || unmanaged "$LAUNCH"
  if [[ -d $APP && -n $(find "$APP" -mindepth 1 -maxdepth 1 -print) && $P_PRESENT == 0 && ! -f $LAUNCH ]]; then
    [[ -f $APP/package.json ]] || unmanaged "$APP"
    JSON_DATA=$(json_parse < "$APP/package.json") && [[ $(jfield name) == envswitch ]] || unmanaged "$APP"
  fi
  os=$(uname -s | tr '[:upper:]' '[:lower:]'); arch=$(uname -m)
  case "$os" in darwin) info 'detected macOS'; if command -v sysctl >/dev/null 2>&1 && [[ $(sysctl -n hw.optional.arm64 2>/dev/null || :) == 1 ]]; then arch=arm64; fi;; linux) if [[ -r /proc/version ]] && grep -qi microsoft /proc/version; then info 'detected WSL/Linux'; else info 'detected Linux'; fi;; *) die "unsupported OS for this installer: $os" 'use scripts/install.ps1 on Windows';; esac
  case "$arch" in aarch64|arm64) TARGET=aarch64;; x86_64|amd64) TARGET=x86_64;; *) die "unsupported architecture for this installer: $arch";; esac
  if [[ $os == darwin ]]; then TARGET="$TARGET-apple-darwin"; else TARGET="$TARGET-unknown-linux-musl"; fi
  if command -v sha256sum >/dev/null 2>&1; then HASH=sha256sum; elif command -v shasum >/dev/null 2>&1; then HASH=shasum; else die 'required installer tool not found: sha256sum or shasum'; fi
  TOKEN=$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')
  ensure_dir "$ROOT"
  if ! (set -o noclobber; printf '%s %s\n' "$TOKEN" "$$" > "$LOCK") 2>/dev/null; then
    if [[ -e $LOCK || -L $LOCK ]]; then die "another envswitch installer may be running: $LOCK" 'retry after it exits; remove the lock only after confirming no installer is running'; fi
    die "envswitch installation failed: cannot create installer lock: $LOCK"
  fi
  LOCKED=1; STAGE=$(mktemp -d "$ROOT/.install-stage-XXXXXXXX"); read_receipt
  [[ -z $SELF || ! -f $SELF ]] || cp "$SELF" "$STAGE/executing-install.sh"
  GH=$(command -v gh || :)
  if [[ $TEST == 1 && $GH != "$HOME/"* && $GH != "${USERPROFILE:-$HOME}/"* ]]; then GH=; fi
  VERIFIER=$GH
  if [[ -n $GH ]] && ! bounded 60 "$STAGE/gh-out" "$STAGE/gh-error" "$GH" auth status; then GH=; fi
  [[ $ATTEST != 1 || -n $VERIFIER ]] || die 'gh is required when attestation verification is requested'
  select_release; download SHA256SUMS; download "$ARCHIVE"; manifest; verify_hash "$STAGE/$ARCHIVE" "$ARCHIVE"
  if [[ $ATTEST == 1 ]]; then info "verifying GitHub attestation for $ARCHIVE"; bounded 60 "$STAGE/gh-out" "$STAGE/gh-error" "$VERIFIER" attestation verify "$STAGE/$ARCHIVE" --repo "$REPO" || die "GitHub attestation verification failed for $ARCHIVE"; fi
  info "extracting $KIND archive"; extract_archive
  for name in install.sh install.ps1; do verify_hash "$TREE/scripts/$name" "$name"; done
  PAYLOAD=$(payload_for "$ROOT" "$KIND" "$TAG" "$TARGET")
  if [[ $KIND == native ]]; then
    STAGED="$STAGE/${PAYLOAD##*/}"; cp "$TREE/envswitch" "$STAGED"; chmod 755 "$STAGED"
    check_payload "$STAGED" native 1 || die "envswitch launcher verification failed: $LAUNCH"
  else
    source_shape "$TREE" || die 'archive did not contain an envswitch source package'
    ensure_bun; launcher source "$TREE/src/cli.ts" > "$STAGE/source-check"; chmod 755 "$STAGE/source-check"
    check_payload "$STAGE/source-check" source || die "envswitch launcher verification failed: $LAUNCH"
  fi
  launcher "$KIND" "$PAYLOAD" > "$STAGE/launcher"; write_receipt
  if [[ $P_PRESENT == 1 && $P_KIND == "$KIND" && $P_TAG == "$TAG" && $P_REPO == "$REPO" && $P_DEST == "$DEST" && $P_PAYLOAD == "$PAYLOAD" && -x $LAUNCH && -f $PAYLOAD ]] && cmp -s "$STAGE/marker" "$APP/.version" && cmp -s "$STAGE/launcher" "$LAUNCH" && cmp -s "$TREE/scripts/install.sh" "$APP/scripts/install.sh" && cmp -s "$TREE/scripts/install.ps1" "$APP/scripts/install.ps1"; then
    if [[ $KIND == native ]]; then cmp -s "$STAGED" "$PAYLOAD" && healthy=1 || :; else [[ -f $APP/package.json && -f $APP/bun.lock && -f $APP/src/cli.ts ]] && healthy=1 || :; fi
    if [[ $healthy == 1 ]] && check_payload "$LAUNCH" "$KIND"; then COMMITTED=1; cleanup; printf 'success: envswitch is already up to date (%s)\n' "$TAG"; return; fi
  fi
  oldtag=$P_TAG
  if [[ -z $oldtag && -f $APP/.version ]]; then entry=$(tr -d '\r\n' < "$APP/.version"); if [[ $entry =~ $VERSION_RE ]]; then oldtag="v${entry#v}"; fi; fi
  outcome=installed
  if [[ ${oldtag:-$CURRENT} == "$TAG" && ( $P_PRESENT == 0 || ( $P_KIND == "$KIND" && ${P_TARGET:-$TARGET} == "$TARGET" ) ) ]]; then outcome=repaired; fi
  ensure_dir "$APP"; ensure_dir "$DEST"
  if [[ $KIND == native ]]; then
    ensure_dir "$APP/bin"; path_ok "$PAYLOAD"
    if [[ -e $PAYLOAD ]]; then cmp -s "$PAYLOAD" "$STAGED" || die "existing native payload differs from verified release: $PAYLOAD" 'close envswitch processes and reinstall using the README instructions; preserve the existing file until recovery is complete'
    else info "installing binary to $PAYLOAD"; mv "$STAGED" "$PAYLOAD"; NEW_PAYLOAD=$PAYLOAD; fi
  else
    existing="$STAGE/old-source"; mkdir "$existing"
    for entry in "$APP"/* "$APP"/.[!.]* "$APP"/..?*; do
      [[ -e $entry || -L $entry ]] || continue
      case "${entry##*/}" in src|node_modules|package.json|bun.lock|README.md|tsconfig.json|scripts|bin|.version|install.json) ;; *) unmanaged "$entry";; esac
    done
    for name in src node_modules package.json bun.lock README.md tsconfig.json; do
      entry="$APP/$name"; [[ -e $entry || -L $entry ]] || continue; path_ok "$entry"
      [[ -z $(find "$entry" ! -type d ! -type f -print) ]] || unmanaged "$entry"
      cp -pR "$entry" "$existing/$name"
    done
    source_shape "$existing" 0 || unmanaged "$APP"
    for name in src node_modules package.json bun.lock README.md tsconfig.json; do
      if [[ -e $TREE/$name ]]; then replace_managed "$APP/$name" "$TREE/$name"
      elif [[ -e $APP/$name ]]; then
        entry=${#J_PATH[@]}; J_PATH[entry]="$APP/$name"; J_BACKUP[entry]="$STAGE/removed-$name"; J_CHANGED[entry]=0
        mv "$APP/$name" "$STAGE/removed-$name"; J_CHANGED[entry]=1
      fi
    done
  fi
  boundary after-payload
  info "installing launcher to $LAUNCH"; replace_managed "$LAUNCH" "$STAGE/launcher" 755
  info 'verifying envswitch launcher'
  [[ -x $LAUNCH ]] || die "launcher was not installed or is not executable: $LAUNCH"
  check_payload "$LAUNCH" "$KIND" || die "envswitch launcher verification failed: $LAUNCH"
  boundary after-launcher
  for name in install.sh install.ps1; do replace_managed "$APP/scripts/$name" "$TREE/scripts/$name"; done
  replace_managed "$APP/install.json" "$STAGE/receipt"; replace_managed "$APP/.version" "$STAGE/marker"
  read_receipt
  if [[ $P_TAG != "$TAG" || $P_PAYLOAD != "$PAYLOAD" ]] || ! cmp -s "$STAGE/marker" "$APP/.version"; then die 'envswitch installation failed: metadata verification'; fi
  boundary after-metadata
  case ":$PATH:" in *":$DEST:"*) profile=;; *) profile=$(shell_profile_path); ensure_profile_path "$DEST" "$profile";; esac
  COMMITTED=1; cleanup
  printf 'success: %s envswitch %s\n' "$outcome" "$TAG"
  if [[ $KIND == native ]]; then info "binary: $PAYLOAD"; else info "source: $APP"; fi
  info "launcher: $LAUNCH"
  if [[ -z $profile ]]; then info "run 'envswitch --help' to verify the installation"
  else info 'restart your terminal, or run this command now:'; info "export PATH=\"$(escaped "$DEST"):\$PATH\""; info "then run 'envswitch --help' to verify the installation"; fi
}
trap unexpected ERR
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
main "$@"
