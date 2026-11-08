//
//  AboutLicence.swift
//  DSPi Console
//
//  WP LIC1: the project's licence notice and its GNU GPL v3 section 7 terms in
//  Settings > About (macOS and iOS), from the core's About table
//  (`aboutJson()`, assets/about.json), as every client shows them.  The terms
//  require the attributions and this notice in About.  No word of them is
//  written here.
//

import SwiftUI
import dspi_core

/// The parts of the core's About table this page shows.
struct AboutTable: Decodable {
    struct CreditLine: Decodable { let text: String }
    struct Term: Decodable { let id: String; let title: String; let text: String }
    struct Licence: Decodable {
        let title: String
        let spdx: String
        let copyright: [String]
        let statement: String
        let termsTitle: String
        let termsNote: String
        let terms: [Term]
        let freedoms: String
        let notices: String

        enum CodingKeys: String, CodingKey {
            case title, spdx, copyright, statement, terms, freedoms, notices
            case termsTitle = "terms_title"
            case termsNote = "terms_note"
        }
    }

    let creditLines: [CreditLine]
    let licence: Licence

    enum CodingKeys: String, CodingKey {
        case creditLines = "credit_lines"
        case licence
    }

    /// The table the core holds, read once.  Nil only for a broken build: a
    /// test holds that it parses.
    static let shared: AboutTable? = parse(aboutJson())

    static func parse(_ json: String) -> AboutTable? {
        try? JSONDecoder().decode(AboutTable.self, from: Data(json.utf8))
    }
}

/// The GPL's own text.
private let gplTextURL = URL(string: "https://www.gnu.org/licenses/gpl-3.0.html")!

/// Settings > About: the licence notice, then the additional terms.
struct AboutLicenceSections: View {
    let licence: AboutTable.Licence

    var body: some View {
        Section {
            ForEach(licence.copyright, id: \.self) { line in
                Text(line)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Text(licence.statement)
                .font(.callout)
                .foregroundStyle(.secondary)
                .textSelection(.enabled)
            Link(destination: gplTextURL) {
                Label("GNU General Public License, version 3", systemImage: "doc.text")
            }
            Text(licence.notices)
                .font(.caption)
                .foregroundStyle(.secondary)
        } header: {
            Label(licence.title, systemImage: "doc.plaintext")
        }

        Section {
            Text(licence.termsNote)
                .font(.callout)
                .foregroundStyle(.secondary)
            ForEach(Array(licence.terms.enumerated()), id: \.element.id) { index, term in
                Text("\(index + 1). \(term.title). \(term.text)")
                    .font(.callout)
                    .textSelection(.enabled)
            }
            Text(licence.freedoms)
                .font(.caption)
                .foregroundStyle(.secondary)
        } header: {
            Label(licence.termsTitle, systemImage: "checkmark.seal")
        }
    }
}
