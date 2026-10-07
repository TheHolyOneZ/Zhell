if ($env.ZHELL_SHELL_INTEGRATION? | default "") == "1" {
    $env.config.shell_integration.osc133 = true
    $env.config.shell_integration.osc7 = true
    $env.config.hooks.pre_execution = ($env.config.hooks.pre_execution? | default [] | append {||
        let cmd = (commandline | str replace -a '\' '\\' | str replace -a ';' '\x3b' | str replace -a "\n" '\x0a')
        print -n $"(ansi -o '633;E;')($cmd)(char bel)"
    })
}
