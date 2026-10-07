__zhell_zdotdir=${ZDOTDIR}
ZDOTDIR=${ZHELL_USER_ZDOTDIR:-$HOME}
[[ -r "$ZDOTDIR/.zlogin" ]] && source "$ZDOTDIR/.zlogin"
ZDOTDIR=${ZHELL_USER_ZDOTDIR:-$HOME}
unset ZHELL_USER_ZDOTDIR __zhell_zdotdir
