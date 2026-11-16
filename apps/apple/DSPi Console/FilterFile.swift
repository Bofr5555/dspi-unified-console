import Foundation

// -- WP SWS4 -- THE OPT-OUT ONLY (`-CoreTransport NO`).  Through a core, File >
// Import Filters and Export Filters are the core's `filtersInspect`,
// `filtersImport` and `filtersExport` (FileMenuActions' "Filter files"
// section in DSPi_ConsoleApp.swift); this file is the Swift reader, writer
// and applier for a view model without a core, and nothing else in the app
// may parse or write a filter file (ci/check.sh, swift step, WP SWS4).

/// Text encoding of a single PEQ band, shared by File > Export Filters and both
/// import parsers (native "# DSPi Console" files and REW-style files).
///
/// One formatter and one parser live here on purpose: the previous per-call-site
/// if-ladders had drifted apart, so exports dropped Q on cuts, shelves and
/// notches, and imports couldn't read back notch, all-pass or Linkwitz bands at
/// all.
///
/// Line grammar (a superset of REW's, so REW files parse unchanged):
///
///     Preamp -6.5 dB
///     Filter  3: ON  PK      Fc   100.0 Hz  Gain  +3.0 dB  Q  1.00  [Bypassed]
///     Filter  4: ON  LT      Fc    40.0 Hz  Q  0.70  Fp    28.0 Hz  Qp  0.71
///     Filter  5: OFF
///     Xover   1: ON  LR4LP   Fc  2000.0 Hz
///
/// Everything after the type token is read as label/value pairs, so units and
/// any extra trailing text other tools emit are ignored rather than misread.
enum FilterFile {

    /// Version stamped into exported files as "# Format: N".  Bumped when the
    /// layout gains sections older builds would silently skip; a file with no
    /// stamp is version 1 (inputs only, PEQ bands only, no preamp).
    ///   1 - PEQ bands per channel, name-keyed input headers.
    ///   2 - index-keyed input headers, all input channels, per-input preamp,
    ///       crossover bands, Linkwitz/bypass round-trip.
    static let formatVersion = 2

    /// Which bank a band line belongs to.  Crossover bands live in a separate
    /// per-output bank (4 bands at wire indices 20..23), so they need their own
    /// keyword rather than sharing "Filter" numbering with the PEQ bands.
    enum Bank: String, CaseIterable {
        case peq = "Filter"
        case crossover = "Xover"
    }

    /// Line keywords accepted on read.  No keyword is a prefix of another, so
    /// the order is presentational.  "Crossover" is the DSPi Console for
    /// Windows spelling of the same bank: the two apps write the
    /// keyword differently, and a file that lost its crossovers silently on the
    /// way across is worse than a slightly wider parser.
    private static let readKeywords: [(keyword: String, bank: Bank)] = [
        ("Crossover", .crossover),
        ("Filter", .peq),
        ("Xover", .crossover),
    ]

    /// One parsed line.  `enabled` is false for "OFF" lines and for shapes this
    /// build doesn't know (REW's "None", a type from newer firmware); the caller
    /// decides whether that means "skip the entry" or "leave the band flat".
    struct Band {
        var bank: Bank
        var params: FilterParams
        var enabled: Bool
    }

    // MARK: - Formatting

    /// `Preamp -6.5 dB`, the input trim applied ahead of the band chain.  Same
    /// spelling REW and AutoEQ use, so their files' preamp is honoured too.
    static func formatPreamp(_ db: Float) -> String {
        String(format: "Preamp %+.1f dB\n", db)
    }

    static func format(bank: Bank = .peq, index: Int, band: FilterParams) -> String {
        // Pad the keyword so "Xover" lines column-align with "Filter" ones.
        let keyword = bank.rawValue.padding(toLength: 6, withPad: " ", startingAt: 0)

        guard let code = band.type.fileCode else {
            return String(format: "%@ %2d: OFF\n", keyword, index)
        }

        let paddedType = code.padding(toLength: 8, withPad: " ", startingAt: 0)
        var line = String(format: "%@ %2d: ON  %@Fc %7.1f Hz", keyword, index, paddedType, band.freq)

        if band.type == .linkwitzTransform {
            // LT repurposes the wire fields: freq/q are the driver's (f0, Q0)
            // and gain carries fp in Hz, with Qp as a sidecar.  Spelled out with
            // distinct labels so the target alignment survives a round-trip.
            line += String(format: "  Q %6.3f  Fp %7.1f Hz  Qp %6.3f", band.q, band.gain, band.qp)
        } else {
            // Three decimals: gain is quantized to 0.001 dB, so a coarser field
            // would silently round every band on an export/import round-trip.
            if band.type.usesGain { line += String(format: "  Gain %+7.3f dB", band.gain) }
            if band.type.usesQ    { line += String(format: "  Q %6.3f", band.q) }
        }

        if band.bypass { line += "  [Bypassed]" }

        return line + "\n"
    }

    // MARK: - Parsing

    /// Reads a "Preamp -6.5 dB" line.  Returns nil for any other line.
    static func parsePreamp(line rawLine: String) -> Float? {
        let line = rawLine.trimmingCharacters(in: .whitespaces)
        guard line.lowercased().hasPrefix("preamp") else { return nil }
        // Tolerates both "Preamp -6.5 dB" and REW's "Preamp: -6.5 dB".
        return line.dropFirst("preamp".count)
            .split(whereSeparator: { $0.isWhitespace || $0 == ":" })
            .compactMap { number(String($0)) }
            .first
    }

    /// Parses one band line.  Returns nil when the line isn't a band entry at
    /// all (headers, comments, preamp, blank lines).
    static func parse(line rawLine: String) -> Band? {
        let line = rawLine.trimmingCharacters(in: .whitespaces)

        // "<keyword> <n>:" - requiring the index keeps unrelated prose (e.g. a
        // "Filters: 5" summary line) from being read as a band.
        guard let match = readKeywords.first(where: {
                  line.lowercased().hasPrefix($0.keyword.lowercased())
              }),
              let colon = line.firstIndex(of: ":") else { return nil }
        let bank = match.bank
        let keywordEnd = line.index(line.startIndex, offsetBy: match.keyword.count)
        guard keywordEnd <= colon else { return nil }
        let indexText = line[keywordEnd..<colon].trimmingCharacters(in: .whitespaces)
        guard Int(indexText) != nil else { return nil }

        var tokens = line[line.index(after: colon)...]
            .split(whereSeparator: { $0.isWhitespace })
            .map(String.init)

        let off = Band(bank: bank, params: FilterParams(type: .flat), enabled: false)
        guard !tokens.isEmpty, tokens.removeFirst().uppercased() == "ON" else { return off }
        guard !tokens.isEmpty else { return off }

        let typeToken = tokens.removeFirst()
        let values = labelledValues(tokens)

        // A crossover band may be written either as a single code ("LR4LP",
        // what this app writes) or as family + shape + slope ("LR  HP ...
        // Slope 24 dB/oct", what the Windows app writes).  Both describe the
        // same firmware type.
        guard let type = FilterType(fileCode: typeToken)
                ?? crossoverType(family: typeToken, shape: tokens.first, slope: values["SLOPE"])
        else { return off }

        var params = FilterParams(type: type)
        var fp: Float? = nil
        for (label, value) in values {
            switch label {
            case "FC":   params.freq = value
            case "GAIN": params.gain = value
            case "Q":    params.q = value
            case "FP":   fp = value
            case "QP":   params.qp = value
            default:     break
            }
        }
        // LT's fp lives in the gain field on the wire; accept either spelling so
        // a file hand-written in the plain "Gain" style still lands correctly.
        if type == .linkwitzTransform, let fp { params.gain = fp }

        // "[Bypassed]" is this app's marker, a bare "BYP" token the Windows
        // app's.  Matched as a whole token so a channel name can't trip it.
        params.bypass = line.uppercased().contains("[BYPASSED]")
            || tokens.contains { $0.uppercased() == "BYP" }

        return Band(bank: bank, params: params, enabled: true)
    }

    /// Compose a crossover type from the family/shape/slope spelling.  Returns
    /// nil when any part is missing or the combination isn't a real filter (an
    /// odd-order Linkwitz-Riley, say).
    private static func crossoverType(family token: String, shape: String?, slope: Float?) -> FilterType? {
        guard let family = crossoverFamily(token), let shape, let slope, slope >= 6 else { return nil }
        let lowPass: Bool
        switch shape.uppercased() {
        case "LP": lowPass = true
        case "HP": lowPass = false
        default:   return nil
        }
        return family.filterType(order: Int(slope) / 6, lowPass: lowPass)
    }

    /// "BES" is this app's short name for Bessel, "Bessel" the Windows one.
    private static func crossoverFamily(_ token: String) -> CrossoverFamily? {
        switch token.uppercased() {
        case "LR":            return .linkwitzRiley
        case "BW":            return .butterworth
        case "BES", "BESSEL": return .bessel
        default:              return nil
        }
    }

    /// Reads the "# Format: N" stamp.  Returns nil for any other line; a file
    /// with no stamp at all is version 1.
    static func parseFormatVersion(line rawLine: String) -> Int? {
        let line = rawLine.trimmingCharacters(in: .whitespaces)
        guard line.lowercased().hasPrefix("# format") else { return nil }
        return line.split(whereSeparator: { $0.isWhitespace || $0 == ":" })
            .compactMap { Int($0) }
            .first
    }

    /// Resolve a DSPi Console for Windows channel header - a bare default name
    /// like "Master L", "SPDIF 2 R", "PDM" or "Input 5" - to a channel here.
    ///
    /// The Windows export keys its sections by name rather than by index, and
    /// always writes the built-in default names (a user rename doesn't reach the
    /// file), so this small table is the whole mapping.  Returns nil for any
    /// header that isn't one of them, including this app's own index-keyed
    /// headers, which the caller has already tried.
    static func windowsChannel(header: String, pdmOutput: Int) -> PresetChannelRef? {
        let name = header.trimmingCharacters(in: .whitespaces).uppercased()

        if name == "MASTER L" { return .input(0) }
        if name == "MASTER R" { return .input(1) }
        if name == "PDM" { return .output(pdmOutput) }

        // "Input N" numbers from 1, so input 3 is wire input 2.
        if name.hasPrefix("INPUT "), let index = Int(name.dropFirst("INPUT ".count)),
           index > BASE_MATRIX_INPUTS, index <= MAX_MATRIX_INPUTS {
            return .input(index - 1)
        }

        // "SPDIF k L" / "SPDIF k R" - output pair k, left then right.
        if name.hasPrefix("SPDIF ") {
            let parts = name.dropFirst("SPDIF ".count).split(separator: " ")
            guard parts.count == 2, let pair = Int(parts[0]), pair >= 1 else { return nil }
            switch parts[1] {
            case "L": return .output((pair - 1) * 2)
            case "R": return .output((pair - 1) * 2 + 1)
            default:  return nil
            }
        }

        return nil
    }

    /// Pairs each non-numeric token with the number that follows it, skipping
    /// unit tokens and anything else that isn't a label/value pair.  A repeated
    /// label keeps its last value, matching the order a line is read in.
    private static func labelledValues(_ tokens: [String]) -> [String: Float] {
        guard tokens.count > 1 else { return [:] }
        var pairs: [String: Float] = [:]
        for i in 0..<(tokens.count - 1) {
            guard number(tokens[i]) == nil, let value = number(tokens[i + 1]) else { continue }
            pairs[tokens[i].uppercased()] = value
        }
        return pairs
    }

    /// Parses a numeric token, tolerating a glued unit ("100Hz") and either
    /// decimal separator.
    ///
    /// Separator rule, matching the Windows app so the two agree on the same
    /// file: when a token carries both '.' and ',' the last one is the decimal
    /// point and the other is thousands grouping; a lone ',' is a decimal point.
    /// That reads "1.234,5" and "0,707" correctly and gives up only on a
    /// thousands-grouped integer like "1,000", which neither app writes.
    private static func number(_ token: String) -> Float? {
        var text = token
        while let last = text.last, !last.isNumber, last != ".", last != "," { text.removeLast() }
        guard !text.isEmpty else { return nil }

        let lastDot = text.lastIndex(of: ".")
        let lastComma = text.lastIndex(of: ",")
        if let lastDot, let lastComma {
            let strip: Character = lastDot > lastComma ? "," : "."
            text = text.replacingOccurrences(of: String(strip), with: "")
        }
        return Float(text.replacingOccurrences(of: ",", with: "."))
    }
}

// MARK: - The opt-out's filter files (-- WP SWS4 --, moved from DSPi_ConsoleApp.swift)

extension FileMenuActions {
    // -- WP SWS4 -- The opt-out's two entry points, the only callers of the
    // reader, writer and applier below in the app.

    /// The opt-out's Import Filters: what the file holds as the picker's
    /// offer, or why it cannot be imported.  Synchronous, main thread.
    static func optOutFilterImport(_ contents: String, vm: DSPViewModel) -> FilterImportStep {
        if contents.hasPrefix("# DSPi Console") {
            // DSPi Console format - pick from the channels it carries
            guard let parsed = parseDSPiFile(contents, vm: vm) else {
                return .refused(AppNotices.file("Not a DSPi Console filter file this version reads."))
            }
            return .offer(FilterImportOffer(prompt: importPrompt(for: parsed),
                                            choices: channelChoices(for: parsed, vm: vm)) { chosen, done in
                let report = applyDSPiFile(parsed, to: chosen, vm: vm)
                done(.result("Filters imported" + report.notes))
            })
        }
        // REW format - pick the channels to put its one band list on
        let parsed = parseREWFile(contents)
        guard !parsed.filters.isEmpty else {
            return .refused(AppNotices.file("No valid filters found in the file."))
        }
        return .offer(FilterImportOffer(prompt: importPrompt(for: parsed),
                                        choices: channelChoices(for: parsed, vm: vm)) { chosen, done in
            let report = applyREWFile(parsed, to: chosen, vm: vm)
            done(.result("Filters imported to \(chosen.count) channel(s)" + report.notes))
        })
    }

    /// The opt-out's Export Filters: the file, from the view model.
    static func optOutFilterExport(vm: DSPViewModel) -> String {
        generateExportString(vm: vm)
    }

    // MARK: - Parsing

    struct ParsedREWFile {
        var filters: [FilterParams] = []
        /// REW and AutoEQ lead their exports with the preamp cut that keeps the
        /// filter set from clipping; nil when the file carries none.
        var preamp: Float? = nil
    }

    static func parseREWFile(_ contents: String) -> ParsedREWFile {
        var parsed = ParsedREWFile()

        for line in contents.components(separatedBy: .newlines) {
            if let preamp = FilterFile.parsePreamp(line: line) {
                parsed.preamp = preamp
                continue
            }
            guard let band = FilterFile.parse(line: line) else { continue }
            // Disabled entries are dropped rather than kept as flat placeholders:
            // a REW file numbers its own slots, so packing the enabled ones is
            // what the user expects when importing into our band layout.
            guard band.enabled else { continue }
            parsed.filters.append(band.params)
        }

        return parsed
    }

    struct ParsedChannelData {
        var filters: [FilterParams]
        var enableState: Bool? // nil = no state info (input, or legacy format)
        // nil (not merely empty) means the file carried no such section, so the
        // import must leave the device's current values alone rather than
        // clearing them - older files have neither.
        var xover: [FilterParams]? = nil
        var preamp: Float? = nil
    }

    struct ParsedDSPiFile {
        var channels: [Int: ParsedChannelData]
        /// Version stamp from the file, or 1 when it predates the stamp.
        var formatVersion: Int
    }

    static func parseDSPiFile(_ contents: String, vm: DSPViewModel = AppState.shared.viewModel) -> ParsedDSPiFile? {
        var result: [Int: ParsedChannelData] = [:]
        var formatVersion = 1
        var currentChannel: Int? = nil

        // Header content is keyed by channel index, never by name, so a renamed
        // channel still round-trips.  The output state suffix is optional so a
        // hand-edited file still loads (the enable state is then left alone).
        let inputPattern = try? NSRegularExpression(pattern: "^Input\\s+(\\d+):")
        let outputPattern = try? NSRegularExpression(pattern: "^Output\\s+(\\d+):.*?(?:\\s*\\((Enabled|Disabled)\\))?\\s*$")

        for line in contents.components(separatedBy: .newlines) {
            if let version = FilterFile.parseFormatVersion(line: line) {
                formatVersion = version
                continue
            }

            // Check for channel header [...]
            if line.hasPrefix("[") && line.hasSuffix("]") {
                let headerContent = String(line.dropFirst().dropLast())
                // A header we don't recognise ends the previous section rather
                // than leaving it open, or its bands would land on the wrong
                // channel.
                currentChannel = nil

                // [Input N: Name]
                if let match = inputPattern?.firstMatch(in: headerContent, options: [], range: NSRange(headerContent.startIndex..., in: headerContent)),
                   let idxRange = Range(match.range(at: 1), in: headerContent),
                   let inputIdx = Int(headerContent[idxRange]),
                   inputIdx >= 0 && inputIdx < vm.chOut1 {
                    currentChannel = inputIdx
                    result[inputIdx] = ParsedChannelData(filters: [], enableState: nil)
                }
                // Legacy input headers (pre-index format): [USB L] / [USB R]
                else if headerContent == "USB L" {
                    currentChannel = 0
                    result[0] = ParsedChannelData(filters: [], enableState: nil)
                } else if headerContent == "USB R" {
                    currentChannel = 1
                    result[1] = ParsedChannelData(filters: [], enableState: nil)
                }
                // [Output N: Name (Enabled/Disabled)]
                else if let match = outputPattern?.firstMatch(in: headerContent, options: [], range: NSRange(headerContent.startIndex..., in: headerContent)),
                        let idxRange = Range(match.range(at: 1), in: headerContent),
                        let outputIdx = Int(headerContent[idxRange]),
                        outputIdx >= 0 && outputIdx <= 8 {
                    let eqCh = vm.eqChannel(forOutput: outputIdx)
                    let enabled = Range(match.range(at: 2), in: headerContent)
                        .map { headerContent[$0] == "Enabled" }
                    currentChannel = eqCh
                    result[eqCh] = ParsedChannelData(filters: [], enableState: enabled)
                }
                // A file from DSPi Console for Windows, whose sections are keyed
                // by the built-in channel name rather than by index.
                else if let ref = FilterFile.windowsChannel(header: headerContent,
                                                            pdmOutput: vm.pdmOutputIndex) {
                    switch ref {
                    case .input(let input) where input < vm.chOut1:
                        currentChannel = input
                        result[input] = ParsedChannelData(filters: [], enableState: nil)
                    case .output(let output) where output < vm.numOutputChannels:
                        let eqCh = vm.eqChannel(forOutput: output)
                        currentChannel = eqCh
                        result[eqCh] = ParsedChannelData(filters: [], enableState: nil)
                    default:
                        break   // a channel this platform doesn't have
                    }
                }
                // Backward compat: old channel names (map to V16 output channels)
                else if headerContent == "Out L" {
                    currentChannel = vm.eqChannel(forOutput: 0)
                    result[vm.eqChannel(forOutput: 0)] = ParsedChannelData(filters: [], enableState: nil)
                } else if headerContent == "Out R" {
                    currentChannel = vm.eqChannel(forOutput: 1)
                    result[vm.eqChannel(forOutput: 1)] = ParsedChannelData(filters: [], enableState: nil)
                } else if headerContent == "Sub" {
                    currentChannel = vm.eqChannel(forOutput: vm.pdmOutputIndex)
                    result[vm.eqChannel(forOutput: vm.pdmOutputIndex)] = ParsedChannelData(filters: [], enableState: nil)
                }
                continue
            }

            guard let channel = currentChannel else { continue }

            if let preamp = FilterFile.parsePreamp(line: line) {
                result[channel]?.preamp = preamp
                continue
            }

            // Parse band line.  Unlike the REW path, an OFF/unknown entry keeps
            // its slot as a flat placeholder so band indices stay aligned with
            // the file.
            guard let band = FilterFile.parse(line: line) else { continue }
            let params = band.enabled ? band.params : FilterParams(type: .flat)

            switch band.bank {
            case .peq:
                result[channel]?.filters.append(params)
            case .crossover:
                var xover = result[channel]?.xover ?? []
                xover.append(params)
                result[channel]?.xover = xover
            }
        }

        return result.isEmpty ? nil : ParsedDSPiFile(channels: result, formatVersion: formatVersion)
    }

    // MARK: - Export

    static func generateExportString(vm: DSPViewModel = AppState.shared.viewModel) -> String {
        let dateFormatter = DateFormatter()
        dateFormatter.dateFormat = "yyyy-MM-dd HH:mm:ss"

        var output = "# DSPi Console Filter Settings\n"
        output += "# Exported: \(dateFormatter.string(from: Date()))\n"
        output += "# Format: \(FilterFile.formatVersion)\n\n"

        // Input channels.  Headers are keyed by index, with the (renameable)
        // channel name carried alongside for readability only.  Only the
        // currently active inputs are written - the inactive ones aren't
        // editable in the UI, so exporting their banks would be noise.
        for eqCh in 0..<vm.numMatrixInputs {
            output += "[Input \(eqCh): \(vm.channelNames[eqCh])]\n"
            output += FilterFile.formatPreamp(vm.preampDB[eqCh])
            for (i, filter) in (vm.channelData[eqCh] ?? []).enumerated() {
                output += FilterFile.format(index: i + 1, band: filter)
            }
            output += "\n"
        }

        // Output channels (platform-aware).  Each carries its PEQ bank and, on
        // crossover-capable firmware, its four crossover bands.
        for outputIdx in 0..<vm.numOutputChannels {
            let eqCh = vm.eqChannel(forOutput: outputIdx)
            let name = vm.channelNames[eqCh]
            let state = vm.outputEnabled[outputIdx] ? "Enabled" : "Disabled"
            output += "[Output \(outputIdx): \(name) (\(state))]\n"
            for (i, filter) in (vm.channelData[eqCh] ?? []).enumerated() {
                output += FilterFile.format(index: i + 1, band: filter)
            }
            if vm.firmwareSupportsCrossover {
                for (i, band) in (vm.xoverData[eqCh] ?? []).enumerated() {
                    output += FilterFile.format(bank: .crossover, index: i + 1, band: band)
                }
            }
            output += "\n"
        }

        return output
    }
}

extension FileMenuActions {
    // MARK: - The opt-out's channel choices

    static func importPrompt(for file: ParsedREWFile) -> String {
        filterImportPrompt(native: false, filters: file.filters.count, preamp: file.preamp)
    }

    static func importPrompt(for file: ParsedDSPiFile) -> String {
        filterImportPrompt(native: true, filters: 0, preamp: nil)
    }

    /// A REW file goes to the active inputs (ticked) or the enabled outputs.
    static func channelChoices(for file: ParsedREWFile, vm: DSPViewModel) -> [ImportChannelChoice] {
        var choices: [ImportChannelChoice] = []
        // Active input channels (checked by default)
        for eqCh in 0..<vm.numMatrixInputs {
            choices.append(ImportChannelChoice(id: eqCh, name: vm.channelNames[eqCh], selected: true))
        }
        // Enabled output channels (unchecked by default)
        for outputIdx in 0..<vm.numOutputChannels where vm.outputEnabled[outputIdx] {
            let eqCh = vm.eqChannel(forOutput: outputIdx)
            choices.append(ImportChannelChoice(id: eqCh, name: vm.channelNames[eqCh], selected: false))
        }
        return choices
    }

    /// A DSPi Console file offers each channel it carries, all ticked.
    static func channelChoices(for file: ParsedDSPiFile, vm: DSPViewModel) -> [ImportChannelChoice] {
        // Show channels in order: inputs then outputs (full unified range).
        (0..<vm.numChannels).compactMap { eqCh in
            guard file.channels[eqCh] != nil else { return nil }
            let name = eqCh < vm.channelNames.count ? vm.channelNames[eqCh] : "Ch \(eqCh)"
            return ImportChannelChoice(id: eqCh, name: name, selected: true)
        }
    }

    // MARK: - Applying

    /// A REW or AutoEQ file's bands on each chosen channel.
    @discardableResult
    static func applyREWFile(_ file: ParsedREWFile, to channels: [Int],
                             vm: DSPViewModel = AppState.shared.viewModel) -> ImportReport {
        var report = ImportReport()
        for eqCh in channels {
            report.merge(applyFilters(file.filters, to: eqCh, vm: vm))
            // Preamp is an input-only trim; outputs have no equivalent, so a
            // REW file's preamp is dropped when targeting one.
            if let preamp = file.preamp {
                if eqCh < vm.chOut1 {
                    vm.setPreampChannel(channel: eqCh, db: preamp)
                } else {
                    report.preampDropped = true
                }
            }
        }
        return report
    }

    /// A DSPi Console file's sections on each chosen channel.
    @discardableResult
    static func applyDSPiFile(_ file: ParsedDSPiFile, to channels: [Int],
                              vm: DSPViewModel = AppState.shared.viewModel) -> ImportReport {
        var report = ImportReport()
        // A file from a newer build may carry sections this one doesn't know
        // about, and they'd be dropped silently otherwise.
        report.newerFormat = file.formatVersion > FilterFile.formatVersion

        for eqCh in channels {
            guard let data = file.channels[eqCh] else { continue }

            report.merge(applyFilters(data.filters, to: eqCh, vm: vm))
            if let preamp = data.preamp, eqCh < vm.chOut1 {
                vm.setPreampChannel(channel: eqCh, db: preamp)
            }
            if let xover = data.xover {
                applyCrossover(xover, to: eqCh, report: &report, vm: vm)
            }
            // Restore enable/disable state for output channels
            if eqCh >= vm.chOut1, let enabled = data.enableState {
                vm.setOutputEnable(output: eqCh - vm.chOut1, enabled: enabled)
            }
        }
        return report
    }

    /// What an import couldn't carry across, so the confirmation can say so
    /// instead of leaving the user to spot it.
    struct ImportReport {
        var truncated = 0                     // filters past the channel's band count
        var unsupported: Set<FilterType> = [] // types this firmware would reject
        var crossoverSkipped = false
        var preampDropped = false
        var newerFormat = false

        mutating func merge(_ other: ImportReport) {
            truncated = max(truncated, other.truncated)
            unsupported.formUnion(other.unsupported)
            crossoverSkipped = crossoverSkipped || other.crossoverSkipped
            preampDropped = preampDropped || other.preampDropped
            newerFormat = newerFormat || other.newerFormat
        }

        var notes: String {
            var lines: [String] = []
            if truncated > 0 {
                lines.append("\(truncated) filter(s) past the end of each channel's band bank were not applied.")
            }
            if !unsupported.isEmpty {
                let names = unsupported.map(\.name).sorted().joined(separator: ", ")
                lines.append("Skipped filter types the connected firmware doesn't support: \(names).")
            }
            if crossoverSkipped {
                lines.append("Crossover settings were skipped - the connected firmware doesn't support crossovers.")
            }
            if preampDropped {
                lines.append("The file's preamp was not applied to output channels, which have no input trim.")
            }
            if newerFormat {
                lines.append("This file was written by a newer version of uConsole for DSPi; some settings may have been ignored.")
            }
            return lines.isEmpty ? "" : "\n\n" + lines.joined(separator: "\n")
        }
    }

    @discardableResult
    private static func applyFilters(_ filters: [FilterParams], to channelIndex: Int, vm: DSPViewModel) -> ImportReport {
        var report = ImportReport()
        let bandCount = vm.channelData[channelIndex]?.count ?? 10
        report.truncated = max(0, filters.count - bandCount)

        for band in 0..<bandCount {
            // Bands past the end of the file are cleared, so an import fully
            // replaces the bank rather than leaving stale filters behind.
            var p = band < filters.count ? filters[band] : FilterParams(type: .flat)
            if isUnsupported(p.type, vm: vm) {
                report.unsupported.insert(p.type)
                p = FilterParams(type: .flat)
            }
            vm.setFilter(ch: channelIndex, band: band, p: p)
        }

        return report
    }

    private static func applyCrossover(_ bands: [FilterParams], to channelIndex: Int, report: inout ImportReport,
                                       vm: DSPViewModel) {
        guard channelIndex >= vm.chOut1 else { return }
        if vm.isDeviceConnected && !vm.firmwareSupportsCrossover {
            report.crossoverSkipped = true
            return
        }

        for band in 0..<DSPViewModel.crossoverBandsPerChannel {
            let p = band < bands.count ? bands[band] : FilterParams(type: .flat)
            vm.setCrossoverBand(ch: channelIndex, localBand: band, p: p)
        }
    }

    /// True when the connected firmware would reject this filter type.
    private static func isUnsupported(_ type: FilterType, vm: DSPViewModel) -> Bool {
        !vm.firmwareSupports(filterType: type)
    }
}
