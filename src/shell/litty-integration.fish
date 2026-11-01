# Litty shell integration for fish, loaded from vendor_conf.d through the
# XDG_DATA_DIRS entry Litty adds: OSC 133 prompt / command marks, OSC 7
# working directory and, per LITTY_SHELL_INTEGRATION_FEATURES, a bar cursor.
status is-interactive; or exit 0
set -q __litty_loaded; and exit 0
set -g __litty_loaded 1

# Do not leak Litty's data dir to programs started from this shell.
if set -q LITTY_SHELL_INTEGRATION_XDG_DIR
    set -l idx (contains -i -- $LITTY_SHELL_INTEGRATION_XDG_DIR $XDG_DATA_DIRS)
    and set -e XDG_DATA_DIRS[$idx]
    test (count $XDG_DATA_DIRS) -eq 0; and set -e XDG_DATA_DIRS
    set -e LITTY_SHELL_INTEGRATION_XDG_DIR
end

function __litty_has
    set -l feats "$LITTY_SHELL_INTEGRATION_FEATURES"
    contains -- $argv[1] (string split , -- "$feats")
end

function __litty_prompt_start --on-event fish_prompt
    set -l ret $status
    if set -q __litty_running
        printf '\e]133;D;%s\a' $ret
        set -e __litty_running
    end
    set -l _litty_pwd (string replace -ra '[[:cntrl:]]' '' -- $PWD)
    printf '\e]7;file://%s%s\a' (string replace -ra '[[:cntrl:]]' '' -- (prompt_hostname)) (string replace -a ' ' '%20' -- $_litty_pwd)
    __litty_has cursor; and printf '\e[5 q'
    printf '\e]133;A\a'
end

function __litty_preexec --on-event fish_preexec
    __litty_has cursor; and printf '\e[0 q'
    printf '\e]133;C\a'
    set -g __litty_running 1
end

# Mark the end of the prompt (start of the command input) after fish_prompt.
if functions -q fish_prompt; and not functions -q __litty_original_fish_prompt
    functions -c fish_prompt __litty_original_fish_prompt
    function fish_prompt
        __litty_original_fish_prompt
        printf '\e]133;B\a'
    end
end


function __litty_yo_emit
    set -l payload (printf '%s' "$argv[1]" | command base64 | string join '')
    printf '\e]778;yo;%s\a' "$payload"
end
function yo
    __litty_yo_emit (string join ' ' -- $argv)
end
function __litty_yo_accept
    set -l line (commandline --current-buffer | string collect)
    if string match -rq '^yo(\s|$)' -- "$line"
        printf '\r\n%s\r\n' "$line"
        commandline --replace ''
        __litty_yo_emit (string replace -r '^yo\s*' '' -- "$line")
    end
    commandline -f execute
end
bind \r __litty_yo_accept
bind \n __litty_yo_accept
bind \e\[24\;8~ repaint

printf '\e]778;init\a'
