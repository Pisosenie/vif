# vif puts this where fish looks for vendor config, to play the greeting; fish 4 marks its
# prompts by itself. Fish's data dirs go back to what they were first.
if set -q VIF_FISH_DIR
    set -l dirs (string split : -- "$XDG_DATA_DIRS")
    set -l mine (contains -i -- $VIF_FISH_DIR $dirs); and set -e dirs[$mine]
    if test (count $dirs) -gt 0
        set -gx XDG_DATA_DIRS (string join : -- $dirs)
    else
        set -e XDG_DATA_DIRS
    end
    set -e VIF_FISH_DIR
end

if status is-interactive; and set -q VIF_GREET
    set -g __vif_greet_with $VIF_GREET
    function __vif_greet --on-event fish_prompt
        functions -e __vif_greet
        $__vif_greet_with greet
        set -e __vif_greet_with
    end
end
set -e VIF_GREET
