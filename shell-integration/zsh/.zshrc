__zhell_zdotdir=${ZDOTDIR}
ZDOTDIR=${ZHELL_USER_ZDOTDIR:-$HOME}
[[ -r "$ZDOTDIR/.zshrc" ]] && source "$ZDOTDIR/.zshrc"
ZDOTDIR=$__zhell_zdotdir
[[ -r "$ZDOTDIR/../zhell.zsh" ]] && source "$ZDOTDIR/../zhell.zsh"
[[ -o login ]] || { ZDOTDIR=${ZHELL_USER_ZDOTDIR:-$HOME}; unset ZHELL_USER_ZDOTDIR; }
