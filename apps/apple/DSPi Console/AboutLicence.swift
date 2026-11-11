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
#if os(macOS)
import AppKit
#endif

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
    let linksNote: String?   // -- WP CLEAN1 -- whose links the About lists
    let licence: Licence

    enum CodingKeys: String, CodingKey {
        case creditLines = "credit_lines"
        case linksNote = "links_note"
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
            #if os(macOS)
            // -- WP REL013M -- the bundled text replaces the web page's link
            if BundledLicenceFiles.url("LICENSES/GPL-3.0.txt") == nil {
                Link(destination: gplTextURL) {
                    Label("GNU General Public License, version 3", systemImage: "doc.text")
                }
            }
            BundledLicenceFiles(spdx: licence.spdx)
            #else
            Link(destination: gplTextURL) {
                Label("GNU General Public License, version 3", systemImage: "doc.text")
            }
            #endif
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

#if os(macOS)
/// -- WP REL013M -- The licence texts in the app's Contents/Resources (NOTICE.txt
/// and LICENSES/, from apps/apple/package-macos.sh, as the Qt .dmg carries them),
/// listed as in the Qt app's About and opened in the user's text viewer.  A
/// build straight from Xcode has none and lists none.
struct BundledLicenceFiles: View {
    let spdx: String

    struct File: Identifiable {
        let title: String
        let detail: String
        let path: String
        var id: String { path }
    }

    var files: [File] {
        [File(title: "GNU General Public License, version 3", detail: spdx, path: "LICENSES/GPL-3.0.txt"),
         File(title: "Rust crates: licences and notices", detail: "The crates the core links", path: "LICENSES/rust-crates.txt"),
         File(title: "NOTICE.txt", detail: "", path: "NOTICE.txt"),
         File(title: "BUNDLED.txt", detail: "", path: "LICENSES/BUNDLED.txt"),
         File(title: "LICENSES", detail: "", path: "LICENSES")]
    }

    static func url(_ path: String) -> URL? {
        guard let url = Bundle.main.resourceURL?.appendingPathComponent(path),
              FileManager.default.fileExists(atPath: url.path) else { return nil }
        return url
    }

    var body: some View {
        ForEach(files.filter { Self.url($0.path) != nil }) { file in
            Button {
                if let url = Self.url(file.path) { NSWorkspace.shared.open(url) }
            } label: {
                HStack {
                    Label(file.title, systemImage: file.path == "LICENSES" ? "folder" : "doc.text")
                    Spacer()
                    if !file.detail.isEmpty {
                        Text(file.detail).foregroundStyle(.secondary)
                    }
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .foregroundStyle(Color.accentColor)
        }
    }
}
#endif
