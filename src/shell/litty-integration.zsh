# Litty shell integration for zsh: OSC 133 prompt / command marks, OSC 7
# working directory and, per LITTY_SHELL_INTEGRATION_FEATURES, a bar cursor
# at the prompt (`cursor`) and the window title (`title`).
[[ -o interactive ]] || return 0
(( ${+_litty_loaded} )) && return 0
typeset -g _litty_loaded=1
typeset -g _litty_running=0

_litty_feats="${LITTY_SHELL_INTEGRATION_FEATURES-}"
_litty_has() { [[ ",${_litty_feats}," == *",$1,"* ]] }

_litty_precmd() {
  local ret=$?
  if (( _litty_running )); then
    builtin print -n "\e]133;D;${ret}\a"
    _litty_running=0
  fi
  if (( ${+_litty_yo_erase} )); then
    builtin print -n "\e[${_litty_yo_erase}A\e[J"
    unset _litty_yo_erase
  fi
  local _litty_pwd="${PWD//[[:cntrl:]]/}"
  builtin print -rn -- $'\e]7;file://'"${HOST//[[:cntrl:]]/}${_litty_pwd// /%20}"$'\a'
  if _litty_has title; then
    local _litty_title="${(%):-%~}"
    builtin print -rn -- $'\e]2;'"${_litty_title//[[:cntrl:]]/}"$'\a'
  fi
  _litty_has cursor && builtin print -n "\e[5 q"
  # Prompt start / end marks, re-added whenever a theme rebuilds PS1.
  [[ $PS1 == *$'\e]133;A'* ]] || PS1=$'%{\e]133;A\a%}'"$PS1"
  [[ $PS1 == *$'\e]133;B'* ]] || PS1="$PS1"$'%{\e]133;B\a%}'
  # Run last, so hooks added later (prompt themes) cannot drop the marks.
  if [[ ${precmd_functions[-1]} != _litty_precmd ]]; then
    precmd_functions=(${precmd_functions:#_litty_precmd} _litty_precmd)
  fi
  return $ret
}

_litty_preexec() {
  _litty_has cursor && builtin print -n "\e[0 q"
  _litty_has title && builtin print -rn -- $'\e]2;'"${1//[[:cntrl:]]/}"$'\a'
  builtin print -n "\e]133;C\a"
  _litty_running=1
}

autoload -Uz add-zsh-hook
add-zsh-hook precmd _litty_precmd
add-zsh-hook preexec _litty_preexec

# `yo` stays in ZLE so natural-language punctuation never becomes shell syntax.
_litty_yo_emit() {
  local payload
  payload=$(builtin printf '%s' "$1" | command base64 | command tr -d '\r\n')
  builtin printf '\e]778;yo;%s\a' "$payload"
}
yo() { _litty_yo_emit "$*"; }
_litty_accept_line() {
  if [[ $BUFFER == yo || $BUFFER == yo[[:space:]]* ]]; then
    local query=${BUFFER#yo}
    query=${query##[[:space:]]#}
    builtin print -s -- "$BUFFER"
    # `zle -I` keeps the typed line on screen but makes accept-line redraw one
    # empty prompt below it. Record that prompt's height so `_litty_precmd` can
    # erase it, and drop its marks so only the real next prompt reports ready.
    local -a rows=("${(@f)${(%)${(e)PS1}}}")
    _litty_yo_erase=${#rows}
    zle -I
    BUFFER=''
    _litty_yo_emit "$query"
    PS1=${PS1//$'%{\e]133;A\a%}'/}
    PS1=${PS1//$'%{\e]133;B\a%}'/}
  fi
  zle .accept-line
}
zle -N accept-line _litty_accept_line
_litty_ai_repaint() { zle reset-prompt; }
zle -N _litty_ai_repaint
bindkey '\e[24;8~' _litty_ai_repaint

builtin printf '\e]778;init\a'
