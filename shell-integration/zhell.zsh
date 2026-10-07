[[ -o interactive ]] || return 0
(( ${+__zhell_loaded} )) && return 0
typeset -g __zhell_loaded=1
autoload -Uz add-zsh-hook

__zhell_urlencode() {
    local s=$1 out= c i
    for (( i = 1; i <= ${#s}; i++ )); do
        c=${s[i]}
        case $c in
            [a-zA-Z0-9/._~-]) out+=$c ;;
            *) out+=$(printf '%%%02X' "'$c") ;;
        esac
    done
    print -rn -- "$out"
}

__zhell_escape() {
    local s=$1
    s=${s//\\/\\\\}
    s=${s//;/\\x3b}
    s=${s//$'\n'/\\x0a}
    s=${s//$'\e'/\\x1b}
    s=${s//$'\a'/\\x07}
    print -rn -- "$s"
}

__zhell_precmd() {
    local ec=$?
    print -n "\e]133;D;$ec\a\e]7;file://${HOST}$(__zhell_urlencode "$PWD")\a"
    if [[ $PS1 != *'133;A'* ]]; then
        PS1=$'%{\e]133;A\a%}'"$PS1"$'%{\e]133;B\a%}'
    fi
}

__zhell_preexec() {
    print -n "\e]633;E;$(__zhell_escape "$1")\a\e]133;C\a"
}

add-zsh-hook precmd __zhell_precmd
add-zsh-hook preexec __zhell_preexec
