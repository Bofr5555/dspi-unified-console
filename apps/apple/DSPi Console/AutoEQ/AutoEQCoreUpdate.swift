//
//  AutoEQCoreUpdate.swift
//  DSPi Console
//
//  WP SW18.  The AutoEQ menu's Update Database through the core (CG6):
//  Rebuild from GitHub, Import File and Reset to Built-in.  The core reads the
//  answers, walks GitHub's listings, parses the profiles and writes the user's
//  database in one step; Swift supplies only the HTTP GET (URLSession) and
//  re-reads the file afterwards through AutoEQManager.
//
//  The iOS AutoEQ menu uses this (iOS/IOSAutoEQMenu.swift).  The Mac's menu
//  keeps its own Swift rebuild for now (AutoEQMenuActions).
//

import Foundation
import dspi_core

// MARK: - The host's fetch

/// CG6's `AutoEqFetch` over URLSession.  The core calls `get` on its own
/// threads, up to 20 at once, never on the main thread; each call blocks
/// until the server answers or the timeout passes.
final class URLSessionAutoEqFetch: AutoEqFetch, @unchecked Sendable {
    private let session: URLSession
    private let timeout: TimeInterval

    init(timeout: TimeInterval = 30) {
        self.timeout = timeout
        let config = URLSessionConfiguration.ephemeral
        config.timeoutIntervalForRequest = timeout
        config.httpMaximumConnectionsPerHost = 20
        session = URLSession(configuration: config)
    }

    func get(request: AutoEqFetchRequest) throws -> AutoEqFetchResponse {
        guard let url = URL(string: request.url), url.scheme == "https" else {
            throw DspiError.Transfer(msg: "not an https address: \(request.url)")
        }
        var urlRequest = URLRequest(url: url, timeoutInterval: timeout)
        urlRequest.setValue(request.userAgent, forHTTPHeaderField: "User-Agent")
        if let accept = request.accept {
            urlRequest.setValue(accept, forHTTPHeaderField: "Accept")
        }
        let done = DispatchSemaphore(value: 0)
        var outcome: Result<(Data, HTTPURLResponse), Error> = .failure(URLError(.unknown))
        let task = session.dataTask(with: urlRequest) { data, response, error in
            if let error {
                outcome = .failure(error)
            } else if let http = response as? HTTPURLResponse {
                outcome = .success((data ?? Data(), http))
            } else {
                outcome = .failure(URLError(.badServerResponse))
            }
            done.signal()
        }
        task.resume()
        done.wait()
        switch outcome {
        case .failure(let error):
            throw DspiError.Transfer(msg: error.localizedDescription)
        case .success(let (body, http)):
            return Self.response(status: http.statusCode, body: body) { http.value(forHTTPHeaderField: $0) }
        }
    }

    /// The core's record of an answer: the status, the body and the three
    /// headers it reads.  A `Retry-After` that is a date counts as absent.
    static func response(status: Int, body: Data, header: (String) -> String?) -> AutoEqFetchResponse {
        AutoEqFetchResponse(
            status: UInt16(clamping: status),
            body: body,
            retryAfterS: header("Retry-After").flatMap { UInt32($0.trimmingCharacters(in: .whitespaces)) },
            rateLimitRemaining: header("X-RateLimit-Remaining").flatMap { UInt32($0.trimmingCharacters(in: .whitespaces)) },
            rateLimitReset: header("X-RateLimit-Reset").flatMap { UInt64($0.trimmingCharacters(in: .whitespaces)) })
    }
}

// -- WP SW18F --
extension URLSessionAutoEqFetch {
    /// The fetch a rebuild uses: URLSession, or in a DEBUG build launched with
    /// `-AutoEQScriptedFetch slow` a scripted GitHub that answers slowly and
    /// never touches the network (UI tests of the rebuild sheet).
    static func forRebuild() -> AutoEqFetch {
        #if DEBUG
        if UserDefaults.standard.string(forKey: "AutoEQScriptedFetch") == "slow" {
            return SlowScriptedAutoEqFetch()
        }
        #endif
        return URLSessionAutoEqFetch()
    }
}

#if DEBUG
/// One source (oratory1990) with one headphone ("SW18 Test Phone"), every
/// other address a 404, each answer after 0.4 s: a rebuild that succeeds in a
/// few seconds unless it is cancelled.
final class SlowScriptedAutoEqFetch: AutoEqFetch, @unchecked Sendable {
    private let api = "https://api.github.com/repos/jaakkopasanen/AutoEq/contents/results/"
    private let raw = "https://raw.githubusercontent.com/jaakkopasanen/AutoEq/master/results/"

    func get(request: AutoEqFetchRequest) throws -> AutoEqFetchResponse {
        Thread.sleep(forTimeInterval: 0.4)
        func answer(_ status: UInt16, _ body: String = "") -> AutoEqFetchResponse {
            AutoEqFetchResponse(status: status, body: Data(body.utf8), retryAfterS: nil,
                                rateLimitRemaining: nil, rateLimitReset: nil)
        }
        switch request.url {
        case api + "oratory1990":
            return answer(200, #"[{"name":"t_over-ear","type":"dir"}]"#)
        case api + "oratory1990/t_over-ear":
            return answer(200, #"[{"name":"SW18 Test Phone","type":"dir"}]"#)
        case raw + "oratory1990/t_over-ear/SW18%20Test%20Phone/SW18%20Test%20Phone%20ParametricEQ.txt":
            return answer(200, "Preamp: -6.5 dB\nFilter 1: ON PK Fc 105 Hz Gain 3.0 dB Q 0.70\n")
        default:
            return answer(404)
        }
    }
}
#endif

// MARK: - Update, import, reset

enum AutoEQCoreUpdate {
    /// What the Update Database dialog says about the database in use.
    static func summary(_ manager: AutoEQManager) -> String {
        "Current database: \(manager.databaseDate ?? "Unknown")\nEntries: \(manager.entries.count)"
    }

    /// The core's words on what a rebuild will fetch and write, for the
    /// confirmation before one starts.
    static func rebuildWarning(_ manager: AutoEQManager) -> String {
        autoeqRebuildWarning(dbPath: manager.userDatabasePath)
    }

    /// Import File: the picked file's bytes become the user's database once
    /// the core has checked them; then the manager re-reads it.  Blocks on
    /// the file: call off the main thread.  The manager is updated on main
    /// before `completion` runs there.
    static func importDatabase(_ data: Data, manager: AutoEQManager,
                               completion: @escaping (Result<AutoEqDatabaseInfo, Error>) -> Void) {
        let path = manager.userDatabasePath
        DispatchQueue.global(qos: .userInitiated).async {
            let result = Result { try autoeqImport(dbPath: path, data: data) }
            DispatchQueue.main.async {
                if case .success = result { manager.reloadDatabase() }
                completion(result)
            }
        }
    }

    /// Reset to Built-in: the core removes the user's database and the
    /// manager goes back to the bundled one.
    static func resetToBuiltIn(_ manager: AutoEQManager,
                               completion: @escaping (Result<Bool, Error>) -> Void) {
        let path = manager.userDatabasePath
        DispatchQueue.global(qos: .userInitiated).async {
            let result = Result { try autoeqResetToBuiltin(dbPath: path) }
            DispatchQueue.main.async {
                if case .success = result { manager.reloadDatabase() }
                completion(result)
            }
        }
    }

    /// Words for an alert from any failure of the calls above.
    static func message(_ error: Error) -> String {
        switch error {
        case DspiError.AutoEq(_, let msg, _): return msg
        case DspiError.Io(let msg): return msg
        case DspiError.Transfer(let msg): return msg
        default: return error.localizedDescription
        }
    }
}

// MARK: - Rebuild from GitHub

/// A rebuild on the core's threads, polled for the progress view.  The core
/// writes the file only after a success, in one step; a cancel, a rate limit
/// or too many failed downloads leave the database as it was.
final class AutoEQCoreRebuild: ObservableObject {
    @Published private(set) var progress = AutoEqRebuildProgress(
        fraction: 0, status: "Starting...", finished: false, succeeded: false, code: "",
        retryAfterS: nil, entries: 0, failedProfiles: 0, failedListings: 0, skippedProfiles: 0)

    private let manager: AutoEQManager
    private var handle: AutoEqRebuild?
    private var timer: Timer?
    private var onFinish: ((AutoEqRebuildProgress) -> Void)?

    init(manager: AutoEQManager) {
        self.manager = manager
    }

    /// Starts it (main thread).  `onFinish` runs once on main when it is over,
    /// after the manager has re-read a written database.
    func start(fetch: AutoEqFetch = URLSessionAutoEqFetch.forRebuild(),
               pollInterval: TimeInterval = 0.25,
               onFinish: @escaping (AutoEqRebuildProgress) -> Void) {
        guard handle == nil else { return }
        self.onFinish = onFinish
        manager.isRebuilding = true
        handle = autoeqRebuildStart(dbPath: manager.userDatabasePath, fetch: fetch)
        let timer = Timer(timeInterval: pollInterval, repeats: true) { [weak self] _ in self?.poll() }
        RunLoop.main.add(timer, forMode: .common)
        self.timer = timer
        poll()
    }

    /// Stops at the next batch; the poll reports it finished (`cancelled`).
    func cancel() {
        handle?.cancel()
    }

    var isRunning: Bool { handle != nil }

    private func poll() {
        guard let handle else { return }
        let now = handle.poll()
        progress = now
        manager.rebuildProgress = Double(now.fraction)
        manager.rebuildStatus = now.status
        guard now.finished else { return }
        timer?.invalidate()
        timer = nil
        self.handle = nil
        manager.isRebuilding = false
        if now.succeeded { manager.reloadDatabase() }
        let finish = onFinish
        onFinish = nil
        finish?(now)
    }

    /// One line for the alert after a rebuild, from the core's record.
    static func summary(_ p: AutoEqRebuildProgress) -> String {
        guard p.succeeded else {
            if p.code == "cancelled" { return "The rebuild was cancelled. The database was not changed." }
            var text = p.status.isEmpty ? "The rebuild failed (\(p.code))." : p.status
            if let wait = p.retryAfterS { text += "\nTry again in \(wait) seconds." }
            return text + "\nThe database was not changed."
        }
        var text = "Database rebuilt successfully!\nEntries: \(p.entries)"
        if p.failedProfiles > 0 { text += "\n\(p.failedProfiles) profiles could not be downloaded." }
        if p.failedListings > 0 { text += "\n\(p.failedListings) folders could not be listed; their profiles are missing." }
        return text
    }
}
