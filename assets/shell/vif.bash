# vif starts bash with this instead of ~/.bashrc: it reads that, then adds OSC 133 marks so
# vif knows where each prompt, command and its output are.
if [[ -r ~/.bashrc ]]; then
    builtin source ~/.bashrc
fi

_vif_status() {
    local ret=$?
    builtin printf '\e]133;D;%s\a' "$ret"
    return $ret
}

_vif_marks() {
    [[ $PS1 == *'133;A'* ]] || PS1='\[\e]133;A\a\]'"$PS1"'\[\e]133;B\a\]'
}

[[ ${PS0-} == *'133;C'* ]] || PS0='\e]133;C\a'"${PS0-}"
PROMPT_COMMAND="_vif_status${PROMPT_COMMAND:+; $PROMPT_COMMAND}; _vif_marks"

# Hosts rarely know xterm-vif, so ssh takes a TERM they do; your own ssh alias or function wins.
if [[ $TERM == xterm-vif ]] && ! declare -F ssh >/dev/null && ! alias ssh >/dev/null 2>&1; then
    ssh() { TERM=xterm-256color command ssh "$@"; }
fi
