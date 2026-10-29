import AppKit
import WebKit
import Foundation
import Darwin

final class AppDelegate: NSObject, NSApplicationDelegate, WKNavigationDelegate, WKUIDelegate, WKDownloadDelegate {
    private var window: NSWindow!
    private var webView: WKWebView!
    private var ownedProcesses: [Process] = []
    private var logHandles: [FileHandle] = []
    private var baseURL: URL?
    private var dataDirectory: URL!
    private var stopping = false
    private var generation = 0
    private var localMode = true
    private var temporary = ProcessInfo.processInfo.environment["PARCOURS_DESKTOP_DATA"] != nil
    private var pendingDownloads: [ObjectIdentifier: NSSavePanel] = [:]

    func applicationDidFinishLaunching(_ notification: Notification) {
        let other = NSRunningApplication.runningApplications(withBundleIdentifier: "fr.parcourspsy.client")
            .first { $0.processIdentifier != ProcessInfo.processInfo.processIdentifier }
        if let other = other {
            other.activate(options: [.activateAllWindows])
            NSApp.terminate(nil)
            return
        }
        NSApp.setActivationPolicy(.regular)
        installMenus()
        let config = WKWebViewConfiguration()
        if temporary { config.websiteDataStore = .nonPersistent() }
        webView = WKWebView(frame: .zero, configuration: config)
        webView.navigationDelegate = self
        webView.uiDelegate = self
        webView.allowsBackForwardNavigationGestures = true
        window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1180, height: 820),
                          styleMask: [.titled, .closable, .miniaturizable, .resizable], backing: .buffered, defer: false)
        window.title = "Parcours Psy"
        window.minSize = NSSize(width: 500, height: 600)
        window.contentView = webView
        window.center()
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        do {
            let env = ProcessInfo.processInfo.environment
            if let override = env["PARCOURS_DESKTOP_DATA"] {
                dataDirectory = URL(fileURLWithPath: override, isDirectory: true)
            } else {
                dataDirectory = try FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask,
                    appropriateFor: nil, create: true).appendingPathComponent("Parcours Psy", isDirectory: true)
            }
            try FileManager.default.createDirectory(at: dataDirectory, withIntermediateDirectories: true,
                attributes: [.posixPermissions: 0o700])
            let remote = temporary ? env["PARCOURS_CLIENT_URL"] : UserDefaults.standard.string(forKey: "serviceURL")
            if let remote = remote, !remote.isEmpty { try startRemote(remote) } else { try startLocal() }
        } catch { showError("Impossible d’ouvrir l’application.", detail: error.localizedDescription) }
    }

    private func installMenus() {
        let bar = NSMenu()
        let appMenu = NSMenu()
        appMenu.addItem(withTitle: "À propos de Parcours Psy", action: #selector(about), keyEquivalent: "")
        appMenu.addItem(.separator())
        appMenu.addItem(withTitle: "Connexion à un serveur…", action: #selector(configureServer), keyEquivalent: ",")
        appMenu.addItem(withTitle: "Utiliser le mode local", action: #selector(useLocal), keyEquivalent: "")
        appMenu.addItem(withTitle: "Ouvrir le dossier de données", action: #selector(openData), keyEquivalent: "")
        appMenu.addItem(.separator())
        appMenu.addItem(withTitle: "Masquer Parcours Psy", action: #selector(NSApplication.hide(_:)), keyEquivalent: "h")
        appMenu.addItem(.separator())
        appMenu.addItem(withTitle: "Quitter Parcours Psy", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        let item = NSMenuItem(); item.submenu = appMenu; bar.addItem(item)
        let edit = NSMenu(title: "Édition")
        for (title, selector, key) in [("Annuler", Selector(("undo:")), "z"), ("Couper", #selector(NSText.cut(_:)), "x"),
            ("Copier", #selector(NSText.copy(_:)), "c"), ("Coller", #selector(NSText.paste(_:)), "v"),
            ("Tout sélectionner", #selector(NSText.selectAll(_:)), "a")] {
            edit.addItem(withTitle: title, action: selector, keyEquivalent: key)
        }
        let editItem = NSMenuItem(title: "Édition", action: nil, keyEquivalent: ""); editItem.submenu = edit; bar.addItem(editItem)
        let view = NSMenu(title: "Affichage")
        view.addItem(withTitle: "Actualiser", action: #selector(refresh), keyEquivalent: "r")
        view.addItem(withTitle: "Page d’accueil", action: #selector(home), keyEquivalent: "0")
        let viewItem = NSMenuItem(title: "Affichage", action: nil, keyEquivalent: ""); viewItem.submenu = view; bar.addItem(viewItem)
        NSApp.mainMenu = bar
    }

    private func validatedURL(_ value: String) throws -> URL {
        guard let url = URL(string: value.trimmingCharacters(in: .whitespacesAndNewlines)),
              let host = url.host, url.user == nil, url.password == nil, url.query == nil, url.fragment == nil,
              url.scheme == "https" || (url.scheme == "http" && ["127.0.0.1", "localhost", "::1"].contains(host)) else {
            throw NSError(domain: "ParcoursPsy", code: 1, userInfo: [NSLocalizedDescriptionKey:
                "Saisissez une adresse HTTPS de l’interface Parcours Psy, ou une adresse HTTP locale. Aucun identifiant ne doit figurer dans l’adresse."])
        }
        return url
    }
    private func origin(_ url: URL) -> String {
        let port = url.port.map { ":\($0)" } ?? ""
        return "\(url.scheme ?? "")://\(url.host ?? "")\(port)"
    }
    private func startRemote(_ address: String) throws {
        let url = try validatedURL(address)
        stopOwned()
        localMode = false; baseURL = url
        window.title = "Parcours Psy — serveur connecté"
        webView.load(URLRequest(url: url))
    }
    private func freePort() throws -> Int {
        let descriptor = socket(AF_INET, SOCK_STREAM, 0)
        guard descriptor >= 0 else { throw posixError() }
        defer { Darwin.close(descriptor) }
        var address = sockaddr_in(); address.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
        address.sin_family = sa_family_t(AF_INET); address.sin_port = 0
        address.sin_addr = in_addr(s_addr: inet_addr("127.0.0.1"))
        let result = withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { Darwin.bind(descriptor, $0, socklen_t(MemoryLayout<sockaddr_in>.size)) }
        }
        guard result == 0 else { throw posixError() }
        var size = socklen_t(MemoryLayout<sockaddr_in>.size)
        let read = withUnsafeMutablePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { getsockname(descriptor, $0, &size) }
        }
        guard read == 0 else { throw posixError() }
        return Int(UInt16(bigEndian: address.sin_port))
    }
    private func posixError() -> NSError { NSError(domain: NSPOSIXErrorDomain, code: Int(errno)) }
    private func startLocal() throws {
        stopOwned()
        localMode = true
        let currentGeneration = generation
        let resources = Bundle.main.resourceURL!
        let runtime = resources.appendingPathComponent("runtime", isDirectory: true)
        let frontend = resources.appendingPathComponent("frontend", isDirectory: true)
        let apiPort = try freePort()
        var webPort = try freePort()
        while apiPort == webPort { webPort = try freePort() }
        let webOrigin = "http://127.0.0.1:\(webPort)"
        let apiOrigin = "http://127.0.0.1:\(apiPort)"
        baseURL = URL(string: webOrigin)!
        window.title = "Parcours Psy — local"
        showLoading()
        var common = ProcessInfo.processInfo.environment
        common["NEXT_TELEMETRY_DISABLED"] = "1"
        common["NODE_ENV"] = "production"
        common["PARCOURS_DB"] = dataDirectory.appendingPathComponent("parcours.sqlite").path
        common["PARCOURS_BIND"] = "127.0.0.1:\(apiPort)"
        common["PARCOURS_API_HOST"] = "127.0.0.1:\(apiPort)"
        common["PARCOURS_FRONTEND_ORIGIN"] = webOrigin
        common["PARCOURS_SECURE_COOKIES"] = "false"
        common["RUST_API_URL"] = apiOrigin
        common["APP_ORIGIN"] = webOrigin
        common["HOSTNAME"] = "127.0.0.1"
        common["PORT"] = String(webPort)
        do {
            try launch(runtime.appendingPathComponent("parcours-psy"), arguments: [], directory: dataDirectory,
                       environment: common, log: "api.log", generation: currentGeneration)
            try launch(runtime.appendingPathComponent("node"), arguments: [frontend.appendingPathComponent("server.js").path],
                       directory: frontend, environment: common, log: "interface.log", generation: currentGeneration)
            checkReadiness(attempt: 0, generation: currentGeneration)
        } catch { stopOwned(); throw error }
    }
    private func launch(_ executable: URL, arguments: [String], directory: URL, environment: [String:String],
                        log: String, generation currentGeneration: Int) throws {
        let logURL = dataDirectory.appendingPathComponent(log)
        // A fresh diagnostic file avoids retaining credentials accidentally emitted by a failed dependency.
        try Data().write(to: logURL, options: .atomic)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: logURL.path)
        let handle = try FileHandle(forWritingTo: logURL); logHandles.append(handle)
        let process = Process(); process.executableURL = executable; process.arguments = arguments
        process.currentDirectoryURL = directory; process.environment = environment
        process.standardOutput = handle; process.standardError = handle; process.standardInput = FileHandle.nullDevice
        process.terminationHandler = { [weak self] _ in
            DispatchQueue.main.async {
                guard let self = self, !self.stopping, self.generation == currentGeneration else { return }
                self.stopOwned()
                self.showError("Un service local s’est arrêté.", detail: "Relancez le mode local depuis le menu Parcours Psy. Les données sont conservées dans le dossier de l’application.")
            }
        }
        try process.run(); ownedProcesses.append(process)
    }
    private func checkReadiness(attempt: Int, generation currentGeneration: Int) {
        guard !stopping, generation == currentGeneration, let baseURL = baseURL else { return }
        if attempt >= 100 {
            stopOwned(); showError("Le démarrage a pris trop de temps.", detail: "Vous pouvez relancer le mode local depuis le menu Parcours Psy.")
            return
        }
        var request = URLRequest(url: baseURL.appendingPathComponent("api/auth/status"))
        request.timeoutInterval = 1; request.cachePolicy = .reloadIgnoringLocalCacheData
        URLSession.shared.dataTask(with: request) { [weak self] data, response, _ in
            DispatchQueue.main.async {
                guard let self = self, !self.stopping, self.generation == currentGeneration else { return }
                if let response = response as? HTTPURLResponse, response.statusCode == 200, let data = data,
                   let json = try? JSONSerialization.jsonObject(with: data) as? [String:Any], json["setup_required"] is Bool {
                    self.webView.load(URLRequest(url: baseURL))
                } else {
                    DispatchQueue.main.asyncAfter(deadline: .now() + 0.25) {
                        self.checkReadiness(attempt: attempt + 1, generation: currentGeneration)
                    }
                }
            }
        }.resume()
    }
    private func showLoading() {
        webView.loadHTMLString("<!doctype html><html lang='fr'><meta name='viewport' content='width=device-width'><body style='margin:0;background:#f5f7f0;color:#356754;font:16px -apple-system;display:grid;place-items:center;height:100vh'><div style='text-align:center'><div style='font-size:46px'>✳</div><h1 style='font-size:28px'>Parcours Psy</h1><p>Ouverture de votre espace local…</p></div></body></html>", baseURL: nil)
    }
    private func showError(_ message: String, detail: String) {
        let alert = NSAlert(); alert.messageText = message; alert.informativeText = detail
        alert.addButton(withTitle: "Fermer"); alert.beginSheetModal(for: window)
    }
    private func stopOwned() {
        generation += 1
        for process in ownedProcesses where process.isRunning { process.terminate() }
        ownedProcesses.removeAll()
        for handle in logHandles { try? handle.close() }
        logHandles.removeAll()
    }
    @objc private func useLocal() {
        if !temporary { UserDefaults.standard.removeObject(forKey: "serviceURL") }
        do { try startLocal() } catch { showError("Impossible de lancer le mode local.", detail: error.localizedDescription) }
    }
    @objc private func configureServer() {
        let alert = NSAlert(); alert.messageText = "Connexion à un serveur Parcours Psy"
        alert.informativeText = "Saisissez l’adresse HTTPS de l’interface Next.js du service. Les comptes et les données de ce serveur seront utilisés. La base locale restera sur ce Mac."
        let field = NSTextField(frame: NSRect(x: 0, y: 0, width: 430, height: 26)); field.placeholderString = "https://votre-service.example"
        if !localMode { field.stringValue = baseURL?.absoluteString ?? "" }
        alert.accessoryView = field; alert.addButton(withTitle: "Se connecter"); alert.addButton(withTitle: "Annuler")
        alert.beginSheetModal(for: window) { [weak self] response in
            guard response == .alertFirstButtonReturn, let self = self else { return }
            do {
                let url = try self.validatedURL(field.stringValue)
                try self.startRemote(url.absoluteString)
                if !self.temporary { UserDefaults.standard.set(url.absoluteString, forKey: "serviceURL") }
            } catch { self.showError("Adresse non valide.", detail: error.localizedDescription) }
        }
    }
    @objc private func openData() { if let dataDirectory = dataDirectory { NSWorkspace.shared.open(dataDirectory) } }
    @objc private func refresh() { if localMode && ownedProcesses.isEmpty { useLocal() } else { webView.reload() } }
    @objc private func home() { if let baseURL = baseURL { webView.load(URLRequest(url: baseURL)) } }
    @objc private func about() {
        let alert = NSAlert(); alert.messageText = "Parcours Psy pour macOS"
        alert.informativeText = "Version 0.3.0 · Apple Silicon\nInterface Next.js, API Rust et stockage local intégrés.\n\nPrototype : utilisez uniquement des données fictives. Les demandes de renouvellement et les démarches juridiques restent des suivis de coordination."
        alert.addButton(withTitle: "Fermer"); alert.beginSheetModal(for: window)
    }
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }
    func applicationWillTerminate(_ notification: Notification) { stopping = true; stopOwned() }
    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        stopping = true; stopOwned(); return .terminateNow
    }
    func webView(_ webView: WKWebView, decidePolicyFor navigationAction: WKNavigationAction,
                 decisionHandler: @escaping (WKNavigationActionPolicy) -> Void) {
        guard let url = navigationAction.request.url else { decisionHandler(.cancel); return }
        if url.scheme == "about" || (baseURL.map { origin(url) == origin($0) } ?? false) || url.scheme == "blob" {
            decisionHandler(navigationAction.shouldPerformDownload ? .download : .allow); return
        }
        if navigationAction.navigationType == .linkActivated && ["https", "mailto", "tel"].contains(url.scheme ?? "") {
            NSWorkspace.shared.open(url)
        }
        decisionHandler(.cancel)
    }
    func webView(_ webView: WKWebView, createWebViewWith configuration: WKWebViewConfiguration,
                 for navigationAction: WKNavigationAction, windowFeatures: WKWindowFeatures) -> WKWebView? {
        if let url = navigationAction.request.url, navigationAction.navigationType == .linkActivated, url.scheme == "https" {
            NSWorkspace.shared.open(url)
        }
        return nil
    }
    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
        if (error as NSError).code != NSURLErrorCancelled {
            showError("Le service est indisponible.", detail: "Vérifiez la connexion au serveur ou relancez le mode local depuis le menu Parcours Psy.")
        }
    }
    func webView(_ webView: WKWebView, navigationAction: WKNavigationAction, didBecome download: WKDownload) { download.delegate = self }
    func webView(_ webView: WKWebView, navigationResponse: WKNavigationResponse, didBecome download: WKDownload) { download.delegate = self }
    func download(_ download: WKDownload, decideDestinationUsing response: URLResponse, suggestedFilename: String,
                  completionHandler: @escaping (URL?) -> Void) {
        let panel = NSSavePanel(); panel.nameFieldStringValue = URL(fileURLWithPath: suggestedFilename).lastPathComponent
        pendingDownloads[ObjectIdentifier(download)] = panel
        panel.beginSheetModal(for: window) { [weak self] result in
            self?.pendingDownloads.removeValue(forKey: ObjectIdentifier(download))
            completionHandler(result == .OK ? panel.url : nil)
        }
    }
}

let app = NSApplication.shared
let delegate = AppDelegate()
app.delegate = delegate
app.run()
