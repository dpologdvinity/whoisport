# Regenerate demo.gif with: cargo build --release && vhs demo.tape
# Needs docker with the busybox image. The demo servers run in a private network
# namespace so list mode shows only them, never the recording machine's real ports.
Output demo.gif
Set Shell "bash"
Set FontSize 18
Set Width 1000
Set Height 600
Set Padding 20
Set Theme "Catppuccin Mocha"
Set TypingSpeed 50ms
Set Framerate 15

Hide
Type "export PATH=$PWD/target/release:$PATH PS1='$ '" Enter
Type "docker run -d --rm --name web -p 8080:80 busybox httpd -f -p 80 >/dev/null" Enter
# uid 1000 shows as "dev": a copy of /etc/passwd is bind-mounted in a private mount
# namespace, so the recording never shows the real user name.
Type "{ grep -v '^[^:]*:[^:]*:1000:' /etc/passwd; echo 'dev:x:1000:1000::/tmp:/bin/sh'; } >/tmp/demo-passwd" Enter
Type "unshare -rnm sh -c 'mount --bind /tmp/demo-passwd /etc/passwd && ip link set lo up && exec unshare -U --map-user=1000 --map-group=1000 bash --norc'" Enter
Type "export PS1='$ '; mkdir -p /tmp/demo-site /tmp/demo-api" Enter
# setsid -f: no parent shell, so nothing prints "Terminated" when they are killed.
Type "cd /tmp/demo-site && setsid -f python3 -m http.server 3000 >/dev/null 2>&1" Enter
Type "cd /tmp/demo-api && setsid -f python3 -m http.server 8000 >/dev/null 2>&1" Enter
Type "setsid -f nc -lk 127.0.0.1 5432 >/dev/null 2>&1 </dev/null" Enter
Type "setsid -f nc -lu 127.0.0.1 5353 >/dev/null 2>&1 </dev/null; cd" Enter
Type "sleep 1; clear" Enter
Show

Type "whoisport" Sleep 300ms Enter
Sleep 2.5s
Type "whoisport 3000" Sleep 300ms Enter
Sleep 3s
Hide
Type "clear" Enter
Show
Type "whoisport 3000 --kill" Sleep 300ms Enter
Sleep 3s
Type "whoisport --udp" Sleep 300ms Enter
Sleep 2.5s

Hide
# Only the namespace's listeners are visible here, so this cannot touch anything else.
Type "whoisport 8000 5432 --kill --force >/dev/null 2>&1; whoisport 5353 --udp --kill --force >/dev/null 2>&1; exit" Enter
Sleep 4s
Type "clear" Enter
Show
Type "whoisport 8080" Sleep 300ms Enter
Sleep 3.5s

Hide
Type "docker stop web >/dev/null" Enter
Sleep 4s
