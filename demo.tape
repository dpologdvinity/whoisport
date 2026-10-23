# Regenerate demo.gif with: cargo build --release && vhs demo.tape
Output demo.gif
Set Shell "bash"
Set FontSize 18
Set Width 1000
Set Height 660
Set Padding 20
Set Theme "Catppuccin Mocha"
Set TypingSpeed 60ms

Hide
Type "export PATH=$PWD/target/release:$PATH PS1='$ '" Enter
Type "cd /tmp/demo-site 2>/dev/null || mkdir -p /tmp/demo-site && cd /tmp/demo-site" Enter
Type "python3 -m http.server 3000 >/dev/null 2>&1 & disown; sleep 1; clear" Enter
Show

Type "whoisport 3000" Sleep 400ms Enter
Sleep 3s
Type "whoisport 3000 --kill" Sleep 400ms Enter
Sleep 3s
Type "whoisport 3000" Sleep 400ms Enter
Sleep 3s
