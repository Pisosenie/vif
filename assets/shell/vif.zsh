# OSC 133 marks, so vif knows where each prompt, command and its output are.
builtin autoload -Uz add-zsh-hook

# Powerlevel10k rebuilds the prompt itself, and marks it when told to, as it does for kitty.
builtin typeset -gri __p9k_force_term_shell_integration=1

_vif_precmd() {
    builtin local ret=$?
    builtin print -n "\e]133;D;$ret\a"
    # Prompt themes rebuild PS1 before each prompt, so the marks go in after them, every time.
    [[ $PS1 == *$'\e]133;A'* ]] || PS1=$'%{\e]133;A\a%}'$PS1
    [[ $PS1 == *$'\e]133;B'* ]] || PS1=$PS1$'%{\e]133;B\a%}'
    if [[ ${precmd_functions[-1]} != _vif_precmd ]]; then
        precmd_functions=(${precmd_functions:#_vif_precmd} _vif_precmd)
    fi
}

_vif_preexec() {
    builtin print -n "\e]133;C\a"
}

add-zsh-hook precmd _vif_precmd
add-zsh-hook preexec _vif_preexec

# Hosts rarely know xterm-vif, so ssh takes a TERM they do; your own ssh alias or function wins.
if [[ $TERM == xterm-vif ]]; then
    ssh() { TERM=xterm-256color command ssh "$@" }
fi
