//
//  AutoEQPreview.swift
//  DSPi Console
//
//  WP P1SW2.  The AutoEQ preview before Apply (P1AQ, PRO1 T6), as Qt's and
//  Android's: a choice of channel pair (inputs first, then outputs), the
//  core's preview of the result (the curve, the level or the suggested trim,
//  what the limits profile would hold, the notes) with nothing sent, then
//  Apply through `autoeq_apply`: a snapshot of the state before, one undo
//  step, the outputs' gain left alone unless asked.  The result is a notice
//  line, never a modal.  Only with the device on the core; otherwise the
//  browser applies as before (inputs 1 and 2).
//

import SwiftUI
import dspi_core

/// One channel pair the profile can go to.
struct AutoEQPair: Hashable, Identifiable {
    let left: UInt8
    let right: UInt8
    let isOutput: Bool
    let label: String
    var id: String { "\(left)-\(right)" }
}

final class AutoEQPreviewModel: ObservableObject, Identifiable {
    let entry: HeadphoneEntry
    let pairs: [AutoEQPair]
    @Published var pair: AutoEQPair? { didSet { if pair != oldValue { preview() } } }
    /// Outputs only: write the profile's preamp as their gain too.
    @Published var setOutputLevel = false { didSet { if setOutputLevel != oldValue { preview() } } }
    @Published private(set) var result: AutoEqPreview?
    @Published private(set) var error: String?
    @Published private(set) var busy = false

    private weak var vm: DSPViewModel?
    private static let work = DispatchQueue(label: "com.dspi.autoeq.preview", qos: .userInitiated)

    init(entry: HeadphoneEntry, vm: DSPViewModel) {
        self.entry = entry
        self.vm = vm
        pairs = Self.pairs(vm)
        pair = pairs.first
        preview()
    }

    /// Inputs first (1-2, 3-4, ...), then the outputs, each pair by name.
    static func pairs(_ vm: DSPViewModel) -> [AutoEQPair] {
        func name(_ ch: Int) -> String { ch < vm.channelNames.count ? vm.channelNames[ch] : "\(ch + 1)" }
        var out: [AutoEQPair] = []
        let inputs = max(2, vm.numMatrixInputs)
        for l in stride(from: 0, to: inputs - 1, by: 2) {
            out.append(AutoEQPair(left: UInt8(l), right: UInt8(l + 1), isOutput: false,
                                  label: "\(name(l)) / \(name(l + 1))"))
        }
        for o in stride(from: 0, to: vm.numOutputChannels - 1, by: 2) {
            let l = vm.chOut1 + o
            out.append(AutoEQPair(left: UInt8(l), right: UInt8(l + 1), isOutput: true,
                                  label: "\(name(l)) / \(name(l + 1))"))
        }
        return out
    }

    /// The database entry as the core's profile.
    static func profile(_ entry: HeadphoneEntry) -> AutoEqProfile {
        AutoEqProfile(id: entry.id, name: entry.displayName, preampDb: Float(entry.preamp),
                      filters: entry.filters.map {
                          AutoEqFilter(kind: $0.type, freq: Float($0.freq), q: Float($0.q), gainDb: Float($0.gain))
                      })
    }

    /// The level choice for the core: inputs take the preamp (the default),
    /// outputs keep their gain unless asked.
    private var applyLevel: Bool? { pair?.isOutput == true ? setOutputLevel : nil }

    var available: Bool { vm?.transport.coreStateSource != nil }

    /// The core's preview: nothing is sent.
    func preview() {
        guard let pair = pair, let source = vm?.transport.coreStateSource else { return }
        let profile = Self.profile(entry)
        let level = applyLevel
        busy = true
        Self.work.async {
            let r = source.call { try $0.autoeqPreview(profile: profile, left: pair.left, right: pair.right,
                                                      applyLevel: level) }
            DispatchQueue.main.async {
                self.busy = false
                switch r {
                case .success(let p): self.result = p; self.error = nil
                case .failure(let e): self.result = nil; self.error = CoreInstaller.line(for: e)
                }
            }
        }
    }

    /// Apply as previewed; `done` on main with the line to show.
    func apply(done: @escaping (Bool) -> Void) {
        guard let pair = pair, let vm = vm else { return done(false) }
        busy = true
        AutoEQPreviewModel.apply(entry, pair: pair, applyLevel: applyLevel, vm: vm) { ok in
            self.busy = false
            done(ok)
        }
    }

    /// `autoeq_apply` to `pair`, then the device's state again; the outcome
    /// on the main window's notice line.
    static func apply(_ entry: HeadphoneEntry, pair: AutoEQPair, applyLevel: Bool?, vm: DSPViewModel,
                      done: @escaping (Bool) -> Void = { _ in }) {
        guard let source = vm.transport.coreStateSource else { return done(false) }
        let profile = Self.profile(entry)
        Self.work.async {
            let r = source.writeAndWait { try $0.autoeqApply(profile: profile, left: pair.left, right: pair.right,
                                                            applyLevel: applyLevel) }
            if case .success = r { vm.applyCoreReread(source) }
            DispatchQueue.main.async {
                switch r {
                case .success(let applied):
                    CoreSnapshots.shared.post(Self.appliedLine(entry, pair: pair, applied), .info)
                    CoreSnapshots.shared.reload()   // the snapshot of the state before
                    done(true)
                case .failure(let e):
                    CoreSnapshots.shared.post(CoreInstaller.line(for: e), .warning)
                    done(false)
                }
            }
        }
    }

    static func appliedLine(_ entry: HeadphoneEntry, pair: AutoEQPair, _ applied: AutoEqApply) -> String {
        var parts = ["AutoEQ \u{201C}\(entry.displayName)\u{201D} applied to \(pair.label): one undo step."]
        parts += applied.preview.notes
        parts += applied.notes
        return parts.joined(separator: " ")
    }

    /// "Level: the inputs' preamp to -6.2 dB." or the trim to set by hand.
    var levelLine: String {
        guard let r = result else { return "" }
        let trim = String(format: "%.1f dB", r.suggestedTrimDb)
        switch r.levelTarget {
        case "input_preamp": return "Level: the inputs' preamp goes to \(trim)."
        case "output_gain": return "Level: the outputs' gain goes to \(trim)."
        default: return "Suggested trim: \(trim). The outputs' gain is kept."
        }
    }
}

/// The preview sheet: the pair, the curve, the lines, Cancel and Apply.
struct AutoEQPreviewView: View {
    @ObservedObject var model: AutoEQPreviewModel
    var onDone: (Bool) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(model.entry.displayName).font(.headline)
            Picker("Channels", selection: $model.pair) {
                ForEach(model.pairs) { p in Text(p.label).tag(Optional(p)) }
            }
            .accessibilityIdentifier("autoeqPair")
            if model.pair?.isOutput == true {
                Toggle("Set the outputs' gain to the profile's preamp", isOn: $model.setOutputLevel)
                    .accessibilityIdentifier("autoeqSetOutputLevel")
            }
            AutoEQCurve(curve: model.result?.channels.first?.curve ?? [])
                .frame(height: 140)
                .accessibilityIdentifier("autoeqPreviewCurve")
            if let error = model.error {
                Text(error).foregroundColor(.orange).font(.callout)
            } else if let r = model.result {
                Text(model.levelLine).font(.callout).accessibilityIdentifier("autoeqLevelLine")
                ForEach(Array(r.notes.enumerated()), id: \.offset) { _, line in
                    Text(line).font(.caption).foregroundColor(.secondary)
                }
                Text("Nothing is sent until Apply. Apply keeps a snapshot of the settings before and is one undo step.")
                    .font(.caption).foregroundColor(.secondary)
            }
            HStack {
                Spacer()
                Button("Cancel") { onDone(false) }
                    .keyboardShortcut(.cancelAction)
                Button("Apply") { model.apply { onDone($0) } }
                    .keyboardShortcut(.defaultAction)
                    .disabled(model.result == nil || model.busy)
                    .accessibilityIdentifier("autoeqApply")
            }
        }
        .padding(16)
        .frame(minWidth: 320, idealWidth: 460)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("autoeqPreview")
    }
}

/// The preview curve: dB over log frequency, 20 Hz to 20 kHz.
struct AutoEQCurve: View {
    let curve: [Double]

    var body: some View {
        GeometryReader { geo in
            let span = max(6, (curve.map { abs($0) }.max() ?? 0).rounded(.up))
            let w = geo.size.width, h = geo.size.height
            let lo = log10(20.0), hi = log10(20000.0)
            ZStack {
                RoundedRectangle(cornerRadius: 6).fill(Color.secondary.opacity(0.08))
                Path { p in p.move(to: CGPoint(x: 0, y: h / 2)); p.addLine(to: CGPoint(x: w, y: h / 2)) }
                    .stroke(Color.secondary.opacity(0.4), lineWidth: 0.5)
                Path { p in
                    var started = false
                    for (i, db) in curve.enumerated() {
                        let f = curveFreq(i: UInt32(i))
                        guard f >= 20, f <= 20000, db.isFinite else { continue }
                        let x = (log10(f) - lo) / (hi - lo) * w
                        let y = h / 2 - db / span * (h / 2)
                        if started { p.addLine(to: CGPoint(x: x, y: y)) } else { p.move(to: CGPoint(x: x, y: y)); started = true }
                    }
                }
                .stroke(Color.accentColor, lineWidth: 2)
                VStack {
                    HStack { Text("+\(Int(span)) dB").font(.caption2).foregroundColor(.secondary); Spacer() }
                    Spacer()
                    HStack { Text("-\(Int(span)) dB").font(.caption2).foregroundColor(.secondary); Spacer() }
                }
                .padding(4)
            }
        }
    }
}
