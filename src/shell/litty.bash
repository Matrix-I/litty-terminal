# Litty shell integration for bash. Litty starts `bash --rcfile <this file>`
# (works on every bash, including macOS's 3.2), so load the login startup files
# as the login shell Litty would otherwise start, then install the hooks.
if [ -n "$LITTY_BASH_INJECT" ]; then
  builtin unset LITTY_BASH_INJECT
  if [ -r /etc/profile ]; then builtin source /etc/profile; fi
  for _litty_f in "$HOME/.bash_profile" "$HOME/.bash_login" "$HOME/.profile"; do
    if [ -r "$_litty_f" ]; then
      builtin source "$_litty_f"
      break
    fi
  done
  builtin unset _litty_f
fi

# OSC 133 prompt / command marks, OSC 7 working directory and, per
# LITTY_SHELL_INTEGRATION_FEATURES, a bar cursor and the window title.
[[ $- == *i* ]] || return 0
[[ -n $_litty_loaded ]] && return 0
_litty_loaded=1

_litty_feats="${LITTY_SHELL_INTEGRATION_FEATURES-}"
_litty_has() { [[ ",${_litty_feats}," == *",$1,"* ]]; }

_litty_yo_redraw=
_litty_yo_query=

_litty_prompt_command() {
  local ret=$?
  _litty_armed=
  local _litty_yo_pending=$_litty_yo_redraw
  _litty_yo_redraw=
  printf '\e]133;D;%s\a' "$ret"
  local _litty_pwd="${PWD//[[:cntrl:]]/}"
  printf '\e]7;file://%s%s\a' "${HOSTNAME//[[:cntrl:]]/}" "${_litty_pwd// /%20}"
  _litty_has title && printf '\e]2;%s\a' "${_litty_pwd/#$HOME/\~}"
  _litty_has cursor && printf '\e[5 q'
  # Prompt start / end marks, re-added whenever something rebuilds PS1.
  [[ $PS1 == *'133;A'* ]] || PS1='\[\e]133;A\a\]'"$PS1"
  [[ $PS1 == *'133;B'* ]] || PS1="$PS1"'\[\e]133;B\a\]'
  if [[ -n $_litty_yo_pending ]]; then
    # Emit the query only now, after the prompt Readline redrew for the
    # abandoned `yo` line is erased, so the next prompt-ready mark is the real one.
    builtin printf '\e[1A\e[J'
    _litty_yo_emit "$_litty_yo_query"
  fi
  return $ret
}

# PS0 (bash 4.4+) runs after a command is read, before it executes.
if (( BASH_VERSINFO[0] > 4 || (BASH_VERSINFO[0] == 4 && BASH_VERSINFO[1] >= 4) )); then
  if _litty_has cursor; then
    PS0='\[\e[0 q\e]133;C\a\]'"$PS0"
  else
    PS0='\[\e]133;C\a\]'"$PS0"
  fi
  if [[ "$(declare -p PROMPT_COMMAND 2>/dev/null)" == "declare -a"* ]]; then
    PROMPT_COMMAND=(_litty_prompt_command "${PROMPT_COMMAND[@]}")
  else
    PROMPT_COMMAND="_litty_prompt_command${PROMPT_COMMAND:+; $PROMPT_COMMAND}"
  fi
elif [[ -z "$(trap -p DEBUG)" ]]; then
  # Older bash (macOS /bin/bash 3.2) has no PS0: a DEBUG trap armed at the end
  # of PROMPT_COMMAND marks the first command the user runs. Skipped when
  # something else (bash-preexec, ...) already owns the trap.
  _litty_armed=
  _litty_arm() { _litty_armed=1; }
  _litty_debug() {
    [[ -n $_litty_armed && $BASH_COMMAND != _litty_prompt_command* ]] || return 0
    _litty_armed=
    _litty_has cursor && printf '\e[0 q'
    printf '\e]133;C\a'
  }
  PROMPT_COMMAND="_litty_prompt_command${PROMPT_COMMAND:+; $PROMPT_COMMAND}; _litty_arm"
  trap '_litty_debug' DEBUG
else
  PROMPT_COMMAND="_litty_prompt_command${PROMPT_COMMAND:+; $PROMPT_COMMAND}"
fi

_litty_yo_emit() {
  local payload
  payload=$(builtin printf '%s' "$1" | command base64 | command tr -d '\r\n')
  builtin printf '\e]778;yo;%s\a' "$payload"
}
# The trigger word comes from the `ai-trigger` config key; empty means off.
_litty_trigger=${LITTY_AI_TRIGGER-yo}
[[ $_litty_trigger =~ ^[A-Za-z_][A-Za-z0-9_-]*$ ]] || _litty_trigger=
if [[ -n $_litty_trigger ]]; then
  eval "$_litty_trigger() { _litty_yo_emit \"\$*\"; }"
fi
_litty_yo_accept() {
  [[ -n $_litty_trigger ]] || return 0
  case $READLINE_LINE in
    "$_litty_trigger"|"$_litty_trigger"[[:space:]]*)
      local original="$READLINE_LINE" query="${READLINE_LINE#"$_litty_trigger"}"
      history -s -- "$original"
      builtin printf '\r\n%s\r\n' "$original"
      READLINE_LINE=''
      READLINE_POINT=0
      # Readline redraws the prompt (marks included) before the bound \C-j
      # accepts the empty line; defer the query to the next prompt command.
      _litty_yo_redraw=1
      _litty_yo_query=$query
      ;;
  esac
}
# READLINE_LINE exists only in Bash 4+. Older Bash delegates interception to
# Litty before Enter reaches Readline, keeping punctuation out of shell syntax.
if (( BASH_VERSINFO[0] >= 4 )); then
  bind -x '"\C-x\C-y":_litty_yo_accept'
  bind '"\C-m":"\C-x\C-y\C-j"'
fi
_litty_ai_repaint() { :; }
bind -x '"\e[24;8~":_litty_ai_repaint'
if (( BASH_VERSINFO[0] >= 4 )); then
  builtin printf '\e]778;init\a'
else
  builtin printf '\e]778;init;direct\a'
fi
