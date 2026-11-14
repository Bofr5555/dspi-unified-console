import XCTest
import dspi_core
@testable import DSPi_Console

/// WP SW18: the AutoEQ menu's Update Database through the core (CG6), as the
/// iOS menu drives it (AutoEQCoreUpdate.swift): import, Reset to Built-in and
/// a rebuild against a scripted GitHub, each re-read by AutoEQManager.  And
/// the iOS app carries no firmware image, while the Mac app still does.
/// Both bundles; no network.
final class AutoEQCoreUpdateSW18Tests: XCTestCase {

    private var directory: URL!

    override func setUpWithError() throws {
        directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("SW18-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    }

    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: directory)
    }

    /// A manager whose user database lives in this test's directory.
    private func manager() -> AutoEQManager {
        AutoEQManager(supportDirectory: directory)
    }

    private static let twoProfiles = """
    {"version":1,"generatedAt":"2026-11-14T10:00:00Z","entryCount":2,"entries":[
     {"id":"sw18/One","manufacturer":"SW18","model":"One","source":"oratory1990","formFactor":"over-ear",
      "preamp":-3.0,"filters":[{"type":"peaking","freq":100,"q":0.7,"gain":2.0}]},
     {"id":"sw18/Two","manufacturer":"SW18","model":"Two","source":"crinacle","formFactor":"in-ear",
      "preamp":-1.0,"filters":[{"type":"lowShelf","freq":105,"q":0.7,"gain":3.0}]}]}
    """

    private func wait<T>(_ what: String, _ start: (@escaping (T) -> Void) -> Void) -> T {
        let done = expectation(description: what)
        var value: T?
        start { value = $0; done.fulfill() }
        wait(for: [done], timeout: 10)
        return value!
    }

    func testSW18ImportThenResetToBuiltInThroughTheCore() throws {
        let m = manager()
        let builtIn = m.entries.count
        XCTAssertGreaterThan(builtIn, 2, "the bundled database")
        XCTAssertFalse(m.hasUserDatabase)

        let imported = wait("import") { AutoEQCoreUpdate.importDatabase(Data(Self.twoProfiles.utf8), manager: m, completion: $0) }
        let info = try imported.get()
        XCTAssertEqual(info.entryCount, 2)
        XCTAssertTrue(info.hasUserDatabase)
        XCTAssertTrue(m.hasUserDatabase)
        XCTAssertEqual(m.entries.map(\.id), ["sw18/One", "sw18/Two"], "the manager re-read the core's file")
        XCTAssertEqual(autoeqDatabaseInfo(dbPath: m.userDatabasePath).entryCount, 2)

        let reset = wait("reset") { AutoEQCoreUpdate.resetToBuiltIn(m, completion: $0) }
        XCTAssertEqual(try reset.get(), true, "there was a user database to remove")
        XCTAssertFalse(m.hasUserDatabase)
        XCTAssertEqual(m.entries.count, builtIn, "back on the bundled database")
    }

    func testSW18AnImportThatIsNotADatabaseChangesNothing() throws {
        let m = manager()
        _ = try wait("first import") { AutoEQCoreUpdate.importDatabase(Data(Self.twoProfiles.utf8), manager: m, completion: $0) }.get()

        let refused = wait("bad import") { AutoEQCoreUpdate.importDatabase(Data("{\"not\": \"a database\"}".utf8), manager: m, completion: $0) }
        guard case .failure(let error) = refused else { return XCTFail("a file that is no database was taken") }
        guard case DspiError.AutoEq(let code, _, _) = error else { return XCTFail("not an AutoEQ error: \(error)") }
        XCTAssertEqual(code, "bad_content")
        XCTAssertFalse(AutoEQCoreUpdate.message(error).isEmpty)
        XCTAssertEqual(m.entries.map(\.id), ["sw18/One", "sw18/Two"], "the database in use stayed")
        XCTAssertEqual(autoeqDatabaseInfo(dbPath: m.userDatabasePath).entryCount, 2, "the file stayed")
    }

    // -- WP SW18F -- F2: the smallest database the core takes loads here too.
    func testSW18FAMinimalDatabaseTheCoreAcceptsLoads() throws {
        let m = manager()
        let imported = wait("import") { AutoEQCoreUpdate.importDatabase(Data(#"{"entries":[{"id":"x/y"}]}"#.utf8), manager: m, completion: $0) }
        XCTAssertEqual(try imported.get().entryCount, 1, "the core took it")
        XCTAssertNil(m.errorMessage, "the manager could not read what the core accepted")
        XCTAssertEqual(m.entries.map(\.id), ["x/y"])
        let entry = try XCTUnwrap(m.entries.first)
        XCTAssertEqual(entry.manufacturer, "")
        XCTAssertEqual(entry.preamp, 0)
        XCTAssertEqual(entry.filters, [])
        // A filter needs its type, as in the core; the rest defaults.
        let db = try JSONDecoder().decode(AutoEQDatabase.self, from: Data(#"{"entries":[{"id":"a","filters":[{"type":"PK"}]}]}"#.utf8))
        XCTAssertEqual(db.entries.first?.filters.first, EmbeddedFilter(type: "PK", freq: 0, q: 0, gain: 0))
        XCTAssertThrowsError(try JSONDecoder().decode(AutoEQDatabase.self, from: Data(#"{"entries":[{"model":"no id"}]}"#.utf8)))
    }

    func testSW18ARebuildFromAScriptedGitHubIsReadBack() {
        let m = manager()
        let fetch = ScriptedGitHub(source: "oratory1990", headphones: ["Sennheiser HD 600"])
        let rebuild = AutoEQCoreRebuild(manager: m)
        let done = wait("rebuild") { finish in rebuild.start(fetch: fetch, pollInterval: 0.05, onFinish: finish) }

        XCTAssertTrue(done.finished)
        XCTAssertTrue(done.succeeded, "\(done.code): \(done.status)")
        XCTAssertEqual(done.entries, 1)
        XCTAssertFalse(m.isRebuilding)
        XCTAssertTrue(m.hasUserDatabase)
        XCTAssertEqual(m.entries.map(\.displayName), ["Sennheiser HD 600"], "the manager re-read the rebuilt file")
        XCTAssertTrue(fetch.userAgents.allSatisfy { $0.hasPrefix("uConsole-for-DSPi/") }, "\(fetch.userAgents)")
        XCTAssertTrue(AutoEQCoreRebuild.summary(done).contains("Entries: 1"))
    }

    func testSW18ARateLimitedRebuildLeavesTheDatabase() {
        let m = manager()
        let builtIn = m.entries.count
        let fetch = ScriptedGitHub(source: "oratory1990", headphones: [], rateLimited: true)
        let rebuild = AutoEQCoreRebuild(manager: m)
        let done = wait("rebuild") { finish in rebuild.start(fetch: fetch, pollInterval: 0.05, onFinish: finish) }

        XCTAssertTrue(done.finished)
        XCTAssertFalse(done.succeeded)
        XCTAssertEqual(done.code, "rate_limited")
        XCTAssertEqual(done.retryAfterS, 60)
        XCTAssertFalse(m.hasUserDatabase, "nothing was written")
        XCTAssertEqual(m.entries.count, builtIn)
        let text = AutoEQCoreRebuild.summary(done)
        XCTAssertTrue(text.contains("60 seconds") && text.contains("not changed"), text)
    }

    func testSW18TheFetchReadsTheRateLimitHeaders() {
        let headers = ["Retry-After": "120", "X-RateLimit-Remaining": "0", "X-RateLimit-Reset": "1700000000"]
        let r = URLSessionAutoEqFetch.response(status: 403, body: Data("x".utf8)) { headers[$0] }
        XCTAssertEqual(r.status, 403)
        XCTAssertEqual(r.retryAfterS, 120)
        XCTAssertEqual(r.rateLimitRemaining, 0)
        XCTAssertEqual(r.rateLimitReset, 1_700_000_000)
        let date = URLSessionAutoEqFetch.response(status: 503, body: Data()) { $0 == "Retry-After" ? "Wed, 21 Oct 2026 07:28:00 GMT" : nil }
        XCTAssertNil(date.retryAfterS, "a date is not seconds")
        XCTAssertThrowsError(try URLSessionAutoEqFetch().get(request: AutoEqFetchRequest(url: "http://example.invalid/", accept: nil, userAgent: "t")),
                             "plain http is refused before any request")
    }

    /// The firmware images: none in the iOS app (an iPhone cannot write them),
    /// both still in the Mac app.  Looks through the built bundle itself, by
    /// extension and folder, so an image under any name is caught (WP SW18F:
    /// the `Firmware` folder is one folder reference, excluded from iOS whole).
    func testSW18FirmwareImagesOnlyInTheMacBundle() throws {
        let files = FileManager.default.enumerator(at: Bundle.main.bundleURL, includingPropertiesForKeys: nil)?
            .compactMap { $0 as? URL } ?? []
        let images = files.filter { $0.pathExtension.lowercased() == "uf2" }
        let folders = files.filter { $0.lastPathComponent == "Firmware" }
        #if os(iOS)
        XCTAssertEqual(images.map(\.lastPathComponent), [], "the iOS bundle carries firmware images")
        XCTAssertEqual(folders.map(\.path), [], "the iOS bundle carries the Firmware folder")
        for chip in BootloaderBoard.Chip.allCases {
            XCTAssertThrowsError(try FirmwareImage.bundled(for: chip))
        }
        #else
        let folder = try XCTUnwrap(Bundle.main.resourceURL).appendingPathComponent("Firmware", isDirectory: true)
        XCTAssertEqual(Set(images.map { $0.deletingLastPathComponent().standardizedFileURL.path }),
                       [folder.standardizedFileURL.path], "the images live in Contents/Resources/Firmware/")
        XCTAssertEqual(images.count, BootloaderBoard.Chip.allCases.count, "one image per chip: \(images.map(\.lastPathComponent))")
        for chip in BootloaderBoard.Chip.allCases {
            let image = try FirmwareImage.bundled(for: chip)
            XCTAssertEqual(image.url.deletingLastPathComponent().standardizedFileURL.path, folder.standardizedFileURL.path)
        }
        #endif
    }
}

/// GitHub as the rebuild sees it, written down in advance: one source with one
/// target holding `headphones`, every other URL a 404.  Or, rate limited, a
/// 403 with the counter at zero for everything.
private final class ScriptedGitHub: AutoEqFetch, @unchecked Sendable {
    private let api = "https://api.github.com/repos/jaakkopasanen/AutoEq/contents/results/"
    private let raw = "https://raw.githubusercontent.com/jaakkopasanen/AutoEq/master/results/"
    private let source: String
    private let headphones: [String]
    private let rateLimited: Bool
    private let lock = NSLock()
    private var agents: [String] = []

    init(source: String, headphones: [String], rateLimited: Bool = false) {
        self.source = source
        self.headphones = headphones
        self.rateLimited = rateLimited
    }

    var userAgents: [String] { lock.lock(); defer { lock.unlock() }; return agents }

    private func dirs(_ names: [String]) -> Data {
        Data(("[" + names.map { "{\"name\":\"\($0)\",\"type\":\"dir\"}" }.joined(separator: ",") + "]").utf8)
    }

    private func answer(_ status: UInt16, _ body: Data = Data(), retryAfter: UInt32? = nil, remaining: UInt32? = nil) -> AutoEqFetchResponse {
        AutoEqFetchResponse(status: status, body: body, retryAfterS: retryAfter, rateLimitRemaining: remaining, rateLimitReset: nil)
    }

    func get(request: AutoEqFetchRequest) throws -> AutoEqFetchResponse {
        lock.lock(); agents.append(request.userAgent); lock.unlock()
        if rateLimited { return answer(403, retryAfter: 60, remaining: 0) }
        let enc = { (s: String) in s.replacingOccurrences(of: " ", with: "%20") }
        switch request.url {
        case api + enc(source):
            return answer(200, dirs(["t_over-ear"]))
        case api + enc(source) + "/t_over-ear":
            return answer(200, dirs(headphones))
        default:
            for h in headphones where request.url == raw + enc(source) + "/t_over-ear/" + enc(h) + "/" + enc(h) + "%20ParametricEQ.txt" {
                return answer(200, Data("Preamp: -6.5 dB\nFilter 1: ON PK Fc 105 Hz Gain 3.0 dB Q 0.70\n".utf8))
            }
            return answer(404)
        }
    }
}
