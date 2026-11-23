// Finder entry for the Beeper fork. No shell, Terminal, bundled secret or Server setup.
import AppKit
import Foundation

struct Connection: Codable {
    let host: String
    let user: String
    let allowWrites: Bool
}

enum LaunchError: Error { case invalid, ssh, target, tunnel, missingBinary }

/// The bundle's Dock presence belongs to the packaged app, whose main
/// executable is this launcher; the real window lives in the child client
/// process. A Dock click can therefore deliver the reopen/activate here,
/// where no window exists. Forward every activation to the client so its
/// window comes forward the same way the client's own reopen handler does.
/// One short line per Dock/activation event in the profile's
/// dock-diagnostic.log (no message or token content); cut back past 32 KiB.
func diag(_ line: String) {
    let url = FileManager.default.homeDirectoryForCurrentUser
        .appendingPathComponent(".zapfast-beeper-prototype/dock-diagnostic.log")
    if let size = (try? FileManager.default.attributesOfItem(atPath: url.path))?[.size] as? Int, size > 32 * 1024 {
        try? FileManager.default.removeItem(at: url)
    }
    let text = "\(Int(Date().timeIntervalSince1970)) launcher \(line)\n"
    if let handle = try? FileHandle(forWritingTo: url) {
        handle.seekToEndOfFile()
        handle.write(Data(text.utf8))
        try? handle.close()
    } else {
        try? text.write(to: url, atomically: false, encoding: .utf8)
    }
}

final class Forwarder: NSObject, NSApplicationDelegate {
    var clientPID: Int32 = 0
    var pendingURLs: [String] = []
    var deliverURL: ((String) -> Void)?
    func application(_ application: NSApplication, open urls: [URL]) {
        for url in urls where url.scheme?.lowercased() == "whatsapp" && url.absoluteString.utf8.count <= 32768 {
            if pendingURLs.count < 32 { pendingURLs.append(url.absoluteString) }
        }
        flushURLs()
    }
    func flushURLs() {
        guard let deliver = deliverURL else { return }
        let urls = pendingURLs
        pendingURLs.removeAll()
        for url in urls { deliver(url) }
    }
    var onClientExit: () -> Void = {}
    func applicationDidFinishLaunching(_ notification: Notification) {
        // The bundle starts as an agent; keep the tunnel owner out of Cmd-Tab.
        NSApp.setActivationPolicy(.accessory)
    }
    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        activateClient()
        return true
    }
    func applicationDidBecomeActive(_ notification: Notification) {
        activateClient()
    }
    func activateClient() {
        diag("activateClient pid=\(clientPID)")
        guard clientPID > 0,
              let running = NSRunningApplication(processIdentifier: clientPID) else { return }
        // macOS 14 made activation cooperative: .activateIgnoringOtherApps no
        // longer has any effect, so the launcher, which the Dock click just
        // activated, yields activation to the client and asks on its behalf.
        if #available(macOS 14.0, *) {
            NSApp.yieldActivation(to: running)
            _ = running.activate(from: NSRunningApplication.current, options: [.activateAllWindows])
        } else {
            running.activate(options: [.activateAllWindows, .activateIgnoringOtherApps])
        }
    }
}
let forwarder = Forwarder()
let manager = FileManager.default
let profile = manager.homeDirectoryForCurrentUser.appendingPathComponent(".zapfast-beeper-prototype")
let configURL = profile.appendingPathComponent("launcher.json")
let app = NSApplication.shared
app.setActivationPolicy(.accessory)
app.delegate = forwarder

func showError() {
    let alert = NSAlert()
    alert.messageText = "Solmu could not connect to the existing Server"
    alert.informativeText = "Check the Mac Studio's SSH address, your existing SSH key access and that Server is running. Close an old tunnel using port 23374 before retrying. No account was reset or linked. No private response was logged."
    alert.runModal()
}

func valid(_ value: String, pattern: String) -> Bool {
    value.range(of: pattern, options: .regularExpression) != nil
}

func configure() -> Connection? {
    let saved = (try? Data(contentsOf: configURL)).flatMap { try? JSONDecoder().decode(Connection.self, from: $0) }
    // Autojoin: a saved connection launches without asking, so opening the app
    // joins the Server directly. The setup alert stays reachable on purpose:
    // pass `--reconfigure` or hold Option while the app opens.
    if let saved,
       !CommandLine.arguments.contains("--reconfigure"),
       !NSEvent.modifierFlags.contains(.option) { return saved }
    let alert = NSAlert()
    alert.messageText = "Connect to your existing Beeper Server"
    alert.informativeText = "Server runs on your other Mac. Use its SSH hostname or Tailscale IP and your SSH username. Existing key access is required; this app never asks for or saves an SSH password. It does not install Server or link networks."
    alert.addButton(withTitle: "Connect")
    alert.addButton(withTitle: "Cancel")
    let box = NSView(frame: NSRect(x: 0, y: 0, width: 380, height: 115))
    let host = NSTextField(string: saved?.host ?? "")
    host.placeholderString = "Server Mac hostname or IP"
    host.frame = NSRect(x: 0, y: 80, width: 380, height: 24)
    let user = NSTextField(string: saved?.user ?? "")
    user.placeholderString = "SSH username on the Server Mac"
    user.frame = NSRect(x: 0, y: 45, width: 380, height: 24)
    let writes = NSButton(checkboxWithTitle: "Enable sending, typing and marking chats read", target: nil, action: nil)
    writes.state = saved?.allowWrites == true ? .on : .off
    writes.frame = NSRect(x: 0, y: 5, width: 380, height: 26)
    box.addSubview(host); box.addSubview(user); box.addSubview(writes)
    alert.accessoryView = box
    app.activate(ignoringOtherApps: true)
    guard alert.runModal() == .alertFirstButtonReturn else { return nil }
    let result = Connection(host: host.stringValue.trimmingCharacters(in: .whitespacesAndNewlines),
                            user: user.stringValue.trimmingCharacters(in: .whitespacesAndNewlines),
                            allowWrites: writes.state == .on)
    guard valid(result.host, pattern: "^[A-Za-z0-9][A-Za-z0-9.:-]{0,252}$"),
          valid(result.user, pattern: "^[A-Za-z_][A-Za-z0-9_.-]{0,63}$") else { return nil }
    return result
}

func ssh(_ connection: Connection, extra: [String]) -> Process {
    let process = Process()
    process.executableURL = URL(fileURLWithPath: "/usr/bin/ssh")
    process.arguments = ["-o", "BatchMode=yes", "-o", "ConnectTimeout=8", "-o", "StrictHostKeyChecking=yes",
                         "-o", "ServerAliveInterval=15", "-o", "ServerAliveCountMax=2"] + extra +
                        ["\(connection.user)@\(connection.host)"]
    process.standardInput = FileHandle.nullDevice
    process.standardError = FileHandle.nullDevice
    return process
}

func stop(_ process: Process) {
    if process.isRunning { process.terminate() }
}

let schedulerHealthScript = (try? String(contentsOf:
    Bundle.main.bundleURL.appendingPathComponent("Contents/Resources/scheduler-health.sh"),
    encoding: .utf8))?.trimmingCharacters(in: .whitespacesAndNewlines) ?? "false"

func launch(_ connection: Connection) throws {
    // Read one named target using the user's pre-existing SSH trust. Fixed command only.
    let query = ssh(connection, extra: [])
    // One fixed command for a macOS or Linux Server host. A Linux host may
    // install the target reader helper from contrib/linux-server (rootless
    // Podman); otherwise the official CLI is found on the usual paths.
    query.arguments! += ["PATH=\"$HOME/.local/bin:$HOME/.npm-global/bin:/home/linuxbrew/.linuxbrew/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin\"; export PATH; if [ -x \"$HOME/.local/bin/zapfast-beeper-target\" ]; then exec \"$HOME/.local/bin/zapfast-beeper-target\"; fi; exec beeper targets show server --json"]
    let output = Pipe()
    query.standardOutput = output
    try query.run()
    let timeout = DispatchWorkItem { stop(query) }
    DispatchQueue.global().asyncAfter(deadline: .now() + 15, execute: timeout)
    defer { timeout.cancel() }
    var bytes = Data()
    while let chunk = try output.fileHandleForReading.read(upToCount: 65536), !chunk.isEmpty {
        guard bytes.count + chunk.count <= 1024 * 1024 else {
            stop(query); throw LaunchError.target
        }
        bytes.append(chunk)
    }
    query.waitUntilExit()
    let tooLarge = bytes.count > 1024 * 1024
    guard query.terminationStatus == 0, !tooLarge,
          let envelope = try JSONSerialization.jsonObject(with: bytes) as? [String: Any],
          envelope["success"] as? Bool != false else { throw LaunchError.target }
    let target = envelope["data"] as? [String: Any] ?? envelope
    guard target["id"] as? String == "server",
          let base = target["baseURL"] as? String,
          let url = URLComponents(string: base), url.scheme == "http", url.host == "127.0.0.1", url.port == 23374,
          url.path.isEmpty || url.path == "/", url.query == nil, url.fragment == nil,
          url.user == nil, url.password == nil,
          let auth = target["auth"] as? [String: Any], let token = auth["accessToken"] as? String,
          !token.isEmpty, token.count <= 16384, !token.contains("\r"), !token.contains("\n")
    else { throw LaunchError.target }

    // Send Later setup: the Server Mac needs the scheduler daemon for
    // scheduled sends while this app is closed. Offer a one-time install of
    // the bundled daemon; nothing is installed or started without approval.
    func offerSchedulerInstall(_ connection: Connection, beeperToken: String) {
        let bundled = Bundle.main.bundleURL.appendingPathComponent("Contents/MacOS/solmu-scheduler")
        guard manager.isExecutableFile(atPath: bundled.path) else { return }
        let declined = profile.appendingPathComponent("scheduler-install-declined")
        let reconfigure = CommandLine.arguments.contains("--reconfigure")
            || NSEvent.modifierFlags.contains(.option)
        guard reconfigure || !manager.fileExists(atPath: declined.path) else { return }
        // The bundled scheduler is a macOS binary and this installer writes a
        // launchd agent. A Linux host installs it on the host with
        // contrib/send-later/install_linux.sh; do not offer what cannot work.
        let host = remote(connection, "uname -s")
        guard host.0 == 0,
              String(decoding: host.1, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines) == "Darwin"
        else { return }
        let check = ssh(connection, extra: [])
        check.arguments! += [schedulerHealthScript + " && echo HEALTHY || true"]
        let checkOut = Pipe()
        check.standardOutput = checkOut
        guard (try? check.run()) != nil else { return }
        let checkTimeout = DispatchWorkItem { stop(check) }
        DispatchQueue.global().asyncAfter(deadline: .now() + 10, execute: checkTimeout)
        let checkData = checkOut.fileHandleForReading.readDataToEndOfFile()
        checkTimeout.cancel()
        check.waitUntilExit()
        guard check.terminationStatus == 0,
              !String(decoding: checkData, as: UTF8.self).contains("HEALTHY") else { return }
        let alert = NSAlert()
        alert.messageText = "Set up Send Later on \(connection.host)?"
        alert.informativeText = "Solmu can copy its scheduler to the Server Mac and start it as a login agent, so scheduled messages send even when this app is closed. You can remove it later (see contrib/send-later). Don't Ask Again hides this until you relaunch while holding Option."
        alert.addButton(withTitle: "Install")
        alert.addButton(withTitle: "Later")
        alert.addButton(withTitle: "Don't Ask Again")
        app.activate(ignoringOtherApps: true)
        let choice = alert.runModal()
        if choice == .alertThirdButtonReturn {
            try? Data().write(to: declined)
            return
        }
        guard choice == .alertFirstButtonReturn else { return }
        installScheduler(connection, bundled: bundled, beeperToken: beeperToken)
    }

    // Runs a script on the Server Mac over the existing SSH trust. The
    // script travels as one argument; only file payloads travel on stdin.
    // stderr comes back too: without it a remote failure is a silent "did
    // not finish" with no way to tell which command the Server Mac refused.
    func remote(_ connection: Connection, _ script: String, stdin: Data? = nil) -> (Int32, Data, Data) {
        let process = ssh(connection, extra: [])
        process.arguments! += ["set -e\n" + script]
        let out = Pipe()
        let err = Pipe()
        process.standardOutput = out
        process.standardError = err
        let input = stdin.map { _ in Pipe() }
        if let input { process.standardInput = input }
        guard (try? process.run()) != nil else { return (-1, Data(), Data()) }
        let timeout = DispatchWorkItem { stop(process) }
        DispatchQueue.global().asyncAfter(deadline: .now() + 30, execute: timeout)
        if let input, let stdin {
            try? input.fileHandleForWriting.write(contentsOf: stdin)
            try? input.fileHandleForWriting.close()
        }
        let data = out.fileHandleForReading.readDataToEndOfFile()
        let errData = err.fileHandleForReading.readDataToEndOfFile()
        timeout.cancel()
        process.waitUntilExit()
        return (process.terminationStatus, data, errData)
    }

    // Mirrors contrib/send-later/install_macos.sh over SSH: bundled daemon,
    // 0600 control token, launchd agent. Tokens never travel on argv.
    func installScheduler(_ connection: Connection, bundled: URL, beeperToken: String) {
        let arch = remote(connection, "uname -s; uname -m")
        guard arch.0 == 0,
              String(decoding: arch.1, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines) == "Darwin\narm64" else {
            let alert = NSAlert()
            alert.messageText = "Send Later needs an Apple Silicon Server Mac"
            alert.informativeText = "The bundled scheduler runs on Apple Silicon only. Send Later stays off; everything else keeps working."
            alert.runModal()
            return
        }
        guard let binaryData = try? Data(contentsOf: bundled) else { return }
        let copy = remote(connection, """
        mkdir -p "$HOME/Library/Application Support/zapfast/bin" "$HOME/Library/LaunchAgents" "$HOME/.config/zapfast"
        chmod 700 "$HOME/Library/Application Support/zapfast" "$HOME/.config/zapfast"
        umask 077
        stage=$(mktemp "$HOME/Library/Application Support/zapfast/bin/scheduler.XXXXXX")
        trap 'rm -f "$stage"' EXIT
        cat > "$stage"
        chmod 755 "$stage"
        mv "$stage" "$HOME/Library/Application Support/zapfast/bin/zapfast-scheduler"
        """, stdin: binaryData)
        guard copy.0 == 0 else { installFailed(connection, stage: "copying the scheduler") ; return }
        let tokens = remote(connection, """
        umask 077
        APP_SUPPORT="$HOME/Library/Application Support/zapfast"
        [ -f "$APP_SUPPORT/scheduler-token" ] || openssl rand -hex 32 > "$APP_SUPPORT/scheduler-token"
        chmod 600 "$APP_SUPPORT/scheduler-token"
        ln -sf "$APP_SUPPORT/scheduler-token" "$HOME/.config/zapfast/scheduler-token"
        [ -f "$APP_SUPPORT/beeper-token" ] && echo HAVE || echo NEED
        """)
        guard tokens.0 == 0 else { installFailed(connection, stage: "writing the control token"); return }
        if String(decoding: tokens.1, as: UTF8.self).contains("NEED") {
            let apiTokenWrite = remote(connection, "umask 077 && cat > \"$HOME/Library/Application Support/zapfast/beeper-token\"",
                       stdin: Data(beeperToken.utf8))
            guard apiTokenWrite.0 == 0 else {
                installFailed(connection, stage: "writing the API token"); return
            }
        }
        let service = remote(connection, """
        APP_SUPPORT="$HOME/Library/Application Support/zapfast"
        PLIST="$HOME/Library/LaunchAgents/com.solmu.scheduler.plist"
        mkdir -p "$APP_SUPPORT" "$HOME/Library/LaunchAgents"
        cat > "$PLIST" <<PLIST
        <?xml version="1.0" encoding="UTF-8"?>
        <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
        <plist version="1.0">
        <dict>
            <key>Label</key>
            <string>com.solmu.scheduler</string>
            <key>ProgramArguments</key>
            <array>
                <string>$APP_SUPPORT/bin/solmu-scheduler</string>
            </array>
            <key>EnvironmentVariables</key>
            <dict>
                <key>SOLMU_LISTEN</key>
                <string>127.0.0.1:23376</string>
                <key>BEEPER_BASE_URL</key>
                <string>http://127.0.0.1:23374</string>
                <key>BEEPER_ACCESS_TOKEN_FILE</key>
                <string>$APP_SUPPORT/beeper-token</string>
                <key>SOLMU_CONTROL_TOKEN_FILE</key>
                <string>$APP_SUPPORT/scheduler-token</string>
                <key>SOLMU_QUEUE_PATH</key>
                <string>$APP_SUPPORT/queue.json</string>
            </dict>
            <key>KeepAlive</key>
            <true/>
            <key>RunAtLoad</key>
            <true/>
            <key>StandardOutPath</key>
            <string>$APP_SUPPORT/scheduler.log</string>
            <key>StandardErrorPath</key>
            <string>$APP_SUPPORT/scheduler.log</string>
        </dict>
        </plist>
        PLIST
        ok=""
        # An already-healthy service is proof the setup is done: leave the
        # running daemon untouched instead of restarting it.
        for _ in 1 2; do
            if \(schedulerHealthScript); then
                ok=1
                break
            fi
            sleep 1
        done
        if [ -z "$ok" ]; then
            # Replace only what actually owns the agent: ask launchd which
            # domain has it, restart that one; on a fresh install prefer the
            # gui domain, then the user domain (a headless Mac reached over
            # SSH can have no gui session), then the legacy load.
            launchctl unload "$PLIST" 2>/dev/null || true
            if launchctl print "gui/$(id -u)/com.solmu.scheduler" >/dev/null 2>&1; then
                launchctl bootout "gui/$(id -u)/com.solmu.scheduler" 2>/dev/null || true
                launchctl bootstrap "gui/$(id -u)" "$PLIST"
            elif launchctl print "user/$(id -u)/com.solmu.scheduler" >/dev/null 2>&1; then
                launchctl bootout "user/$(id -u)/com.solmu.scheduler" 2>/dev/null || true
                launchctl bootstrap "user/$(id -u)" "$PLIST"
            else
                launchctl bootstrap "gui/$(id -u)" "$PLIST" 2>/dev/null || launchctl bootstrap "user/$(id -u)" "$PLIST" 2>/dev/null || launchctl load "$PLIST" 2>&1 | tail -n 2 >&2
            fi
            for _ in 1 2 3 4 5 6 7 8 9 10; do
                if \(schedulerHealthScript); then
                    ok=1
                    break
                fi
                sleep 1
            done
        fi
        if [ -z "$ok" ]; then
            echo "scheduler still not healthy; last scheduler.log lines:" >&2
            tail -n 5 "$APP_SUPPORT/scheduler.log" >&2 2>/dev/null || true
        fi
        [ -n "$ok" ] && echo RUNNING
        """)
        if service.0 == 0, String(decoding: service.1, as: UTF8.self).contains("RUNNING") {
            let alert = NSAlert()
            alert.messageText = "Send Later is set up"
            alert.informativeText = "The scheduler is running on \(connection.host) as a login agent."
            alert.runModal()
        } else {
            let stderrText = String(decoding: service.2, as: UTF8.self)
            let lines = stderrText.split(separator: "\n").map(String.init).filter { !$0.trimmingCharacters(in: .whitespaces).isEmpty }
            let detail = String(lines.suffix(3).joined(separator: "\n").prefix(400))
            installFailed(connection, stage: "starting the scheduler service", detail: detail)
        }
    }

    func installFailed(_ connection: Connection, stage: String, detail: String = "") {
        let alert = NSAlert()
        alert.messageText = "Send Later setup did not finish"
        var text = "Setup stopped while \(stage) on \(connection.host). Anything already copied stays there and is reused on a safe retry (relaunch while holding Option), or see contrib/send-later for manual install and removal."
        let trimmed = detail.trimmingCharacters(in: .whitespacesAndNewlines)
        if !trimmed.isEmpty {
            text += "\n\nThe Server Mac said: \(trimmed)"
        }
        alert.informativeText = text
        alert.runModal()
    }

    // The remote ready marker is emitted only after SSH has established forwarding.
    // Merely checking isRunning before authentication could send a token to an old listener.
    // Send Later rides the same SSH trust: forward the scheduler socket too,
    // but never let an occupied local 23376 break the Server connection.
    func openTunnel(_ forwards: [String]) -> (Process, Pipe)? {
        var extra = ["-T", "-o", "ExitOnForwardFailure=yes"]
        for forward in forwards { extra += ["-L", forward] }
        let tunnel = ssh(connection, extra: extra)
        tunnel.arguments! += ["printf READY; cat >/dev/null"]
        let hold = Pipe()
        let ready = Pipe()
        tunnel.standardInput = hold
        tunnel.standardOutput = ready
        do { try tunnel.run() } catch { return nil }
        let tunnelTimeout = DispatchWorkItem { stop(tunnel) }
        DispatchQueue.global().asyncAfter(deadline: .now() + 15, execute: tunnelTimeout)
        var marker = Data()
        while marker.count < 5 {
            guard let chunk = try? ready.fileHandleForReading.read(upToCount: 5 - marker.count), !chunk.isEmpty else { break }
            marker.append(chunk)
        }
        tunnelTimeout.cancel()
        guard tunnel.isRunning, marker == Data("READY".utf8) else { stop(tunnel); return nil }
        return (tunnel, hold)
    }
    let forwards = ["127.0.0.1:23374:127.0.0.1:23374", "127.0.0.1:23376:127.0.0.1:23376"]
    let opened: (Process, Pipe, Bool)?
    if let both = openTunnel(forwards) {
        opened = (both.0, both.1, true)
    } else if let apiOnly = openTunnel(Array(forwards.prefix(1))) {
        opened = (apiOnly.0, apiOnly.1, false)
    } else {
        opened = nil
    }
    guard let (tunnel, hold, schedulerForwarded) = opened else { throw LaunchError.tunnel }
    defer { stop(tunnel) }
    _ = hold
    // Clear a token from a previous Server before either sync or API-only
    // fallback. If removal fails, do not launch a client that could use it.
    let cachedSchedulerToken = profile.appendingPathComponent("config/scheduler-token")
    if manager.fileExists(atPath: cachedSchedulerToken.path) {
        try manager.removeItem(at: cachedSchedulerToken)
    }

    offerSchedulerInstall(connection, beeperToken: token)

    // Best effort: the scheduler's control token lives only on the Server Mac
    // and reaches this Mac through the same SSH trust, 0600 on both ends.
    // Only with its forward actually up: without it a local listener could be
    // anything, and the app would send the token there. Without a token the
    // app simply keeps Send Later unavailable.
    if schedulerForwarded {
    let tokenQuery = ssh(connection, extra: [])
    tokenQuery.arguments! += ["cat \"$HOME/Library/Application Support/zapfast/scheduler-token\" 2>/dev/null || cat \"$HOME/.config/zapfast/scheduler-token\" 2>/dev/null || true"]
    let tokenOut = Pipe()
    tokenQuery.standardOutput = tokenOut
    if (try? tokenQuery.run()) != nil {
        let tokenTimeout = DispatchWorkItem { stop(tokenQuery) }
        DispatchQueue.global().asyncAfter(deadline: .now() + 10, execute: tokenTimeout)
        let tokenData = tokenOut.fileHandleForReading.readDataToEndOfFile()
        tokenTimeout.cancel()
        tokenQuery.waitUntilExit()
        if tokenQuery.terminationStatus == 0,
           let schedulerToken = String(data: tokenData, encoding: .utf8)?.trimmingCharacters(in: .whitespacesAndNewlines),
           !schedulerToken.isEmpty, schedulerToken.count <= 256,
           valid(schedulerToken, pattern: "^[A-Za-z0-9._~+-]+$") {
            let configDir = profile.appendingPathComponent("config")
            try? manager.createDirectory(at: configDir, withIntermediateDirectories: true,
                                         attributes: [.posixPermissions: 0o700])
            try? schedulerToken.write(to: configDir.appendingPathComponent("scheduler-token"),
                                      atomically: true, encoding: .utf8)
            try? manager.setAttributes([.posixPermissions: 0o600],
                                       ofItemAtPath: configDir.appendingPathComponent("scheduler-token").path)
        }
    }
    }
    let binary = Bundle.main.bundleURL.appendingPathComponent("Contents/MacOS/solmu-client")
    guard manager.isExecutableFile(atPath: binary.path) else { throw LaunchError.missingBinary }
    try manager.createDirectory(at: profile, withIntermediateDirectories: true,
                                attributes: [.posixPermissions: 0o700])
    // Only SSH host/user and the explicit write-mode choice persist. Never the API token.
    try JSONEncoder().encode(connection).write(to: configURL, options: .atomic)
    try manager.setAttributes([.posixPermissions: 0o600], ofItemAtPath: configURL.path)
    let client = Process()
    client.executableURL = binary
    client.arguments = Array(CommandLine.arguments.dropFirst()).filter { !$0.hasPrefix("-psn_") }
    var env = ProcessInfo.processInfo.environment
    for key in ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "http_proxy", "https_proxy", "all_proxy"] { env.removeValue(forKey: key) }
    env["NO_PROXY"] = "127.0.0.1,localhost,::1"
    env["no_proxy"] = "127.0.0.1,localhost,::1"
    env["SOLMU_BACKEND"] = "beeper"
    env["SOLMU_PROFILE_ROOT"] = profile.path
    env["BEEPER_DESKTOP_BASE_URL"] = "http://127.0.0.1:23374"
    env["BEEPER_ACCESS_TOKEN"] = token
    env["BEEPER_READONLY"] = connection.allowWrites ? "0" : "1"
    env["BEEPER_SIGN_IN"] = "0"
    client.environment = env
    client.standardOutput = FileHandle.nullDevice
    client.standardError = FileHandle.nullDevice
    try client.run()
    forwarder.clientPID = client.processIdentifier
    // Forward only as a single argument to the existing instance protocol.
    // No shell, automatic sends, raw URL logging or new tunnel per link.
    forwarder.deliverURL = { url in
        let helper = Process()
        helper.executableURL = binary
        helper.arguments = [url]
        helper.environment = env
        helper.standardOutput = FileHandle.nullDevice
        helper.standardError = FileHandle.nullDevice
        do { try helper.run() } catch { NSSound.beep() }
    }
    forwarder.flushURLs()
    client.terminationHandler = { _ in
        DispatchQueue.main.async { NSApp.terminate(nil) }
    }
    Timer.scheduledTimer(withTimeInterval: 0.25, repeats: true) { _ in
        if !tunnel.isRunning {
            stop(client)
            DispatchQueue.main.async { NSApp.terminate(nil) }
        }
    }
    app.run()
}

if let connection = configure() {
    do { try launch(connection) } catch { showError(); exit(1) }
}
