//
//  HubBrowserGraceDisc2Tests.swift
//  DSPi ConsoleTests
//
//  WP DISC2: a service that leaves the browse results stays listed for the
//  removal grace, as in the core's browser and on Android.
//

import XCTest
import Network
@testable import DSPi_Console

@MainActor
final class HubBrowserGraceDisc2Tests: XCTestCase {

    private func txt(_ hid: String, n: Int = 0) -> NWTXTRecord {
        NWTXTRecord(["hid": hid, "kind": "console", "auth": "none", "n": "\(n)"])
    }

    private func endpoint(_ port: UInt16) -> NWEndpoint {
        .hostPort(host: .ipv4(IPv4Address("192.168.1.20")!), port: NWEndpoint.Port(rawValue: port)!)
    }

    private func wait(_ seconds: TimeInterval) {
        let e = expectation(description: "wait")
        DispatchQueue.main.asyncAfter(deadline: .now() + seconds) { e.fulfill() }
        wait(for: [e], timeout: seconds + 2)
    }

    func testDisc2AServiceThatComesStraightBackNeverLeavesTheList() {
        let browser = HubBrowser(ownHubID: nil)
        browser.removalGrace = 0.5
        browser.ingest([(name: "Studio", txt: txt("hub-1", n: 1), endpoint: endpoint(11915))])
        browser.ingest([])
        XCTAssertEqual(browser.hubs.map { $0.name }, ["Studio"], "listed through the gap")
        browser.ingest([(name: "Studio", txt: txt("hub-1", n: 2), endpoint: endpoint(11915))])
        wait(0.8)
        XCTAssertEqual(browser.hubs.map { $0.deviceCount }, [2], "still there, with the new data")
    }

    func testDisc2AServiceThatStaysAwayLeavesAfterTheGrace() {
        let browser = HubBrowser(ownHubID: nil)
        browser.removalGrace = 0.3
        browser.ingest([(name: "Studio", txt: txt("hub-1"), endpoint: endpoint(11915))])
        browser.ingest([])
        XCTAssertEqual(browser.hubs.count, 1)
        wait(0.6)
        XCTAssertTrue(browser.hubs.isEmpty)
    }

    func testDisc2OurOwnHubAndAZeroGraceGoAtOnce() {
        let own = "94146af2-2bc6-4bca-bb14-6bd911812737"
        let browser = HubBrowser(ownHubID: own)
        browser.ingest([(name: "This Mac", txt: txt("not-yet"), endpoint: endpoint(11915))])
        browser.ingest([(name: "This Mac", txt: txt(own), endpoint: endpoint(11915))])
        XCTAssertTrue(browser.hubs.isEmpty, "ours, not a flap")

        let immediate = HubBrowser(ownHubID: nil)
        immediate.removalGrace = 0
        immediate.ingest([(name: "Studio", txt: txt("hub-1"), endpoint: endpoint(11915))])
        immediate.ingest([])
        XCTAssertTrue(immediate.hubs.isEmpty)
    }

    func testDisc2StopForgetsAHubInItsGrace() {
        let browser = HubBrowser(ownHubID: nil)
        browser.ingest([(name: "Studio", txt: txt("hub-1"), endpoint: endpoint(11915))])
        browser.ingest([])
        browser.stop()
        XCTAssertTrue(browser.hubs.isEmpty)
        XCTAssertEqual(browser.removalGrace, 3, "the core's and Android's grace")
    }
}
