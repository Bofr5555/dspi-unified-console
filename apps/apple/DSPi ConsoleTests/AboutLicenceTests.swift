import XCTest
@testable import DSPi_Console

/// WP LIC1: About shows the core's licence notice and its three section 7
/// terms, and the attribution lines the first term names.  No device.
final class AboutLicenceTests: XCTestCase {

    func testLic1TheCoreTableParsesWithTheLicenceAndThreeTerms() throws {
        let table = try XCTUnwrap(AboutTable.shared)
        XCTAssertEqual(table.licence.spdx, "GPL-3.0-only")
        XCTAssertTrue(table.licence.statement.contains("either version 3 of the License, or (at your option) any later version"))
        XCTAssertEqual(table.licence.terms.map(\.id), ["attributions", "modified_versions", "no_trademark_rights"])
        let texts = table.creditLines.map(\.text)
        XCTAssertTrue(texts.contains("DSPi and its firmware by Weeb Labs"))
        for line in texts {
            XCTAssertTrue(table.licence.terms[0].text.contains("\"\(line)\""), line)
        }
    }

    func testLic1AnythingElseIsNoTable() {
        XCTAssertNil(AboutTable.parse("not json"))
        XCTAssertNil(AboutTable.parse("{}"))
    }
}
