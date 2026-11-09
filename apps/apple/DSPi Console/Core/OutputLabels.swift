//
//  OutputLabels.swift
//  DSPi Console
//
//  BUG1 (owner decision 2026-11-09).  What each output is called: the name
//  the user set, else the core's label from the matrix and the output's
//  filters ("Out 1 · R · Low") while the device still has the firmware's
//  default name for the slot ("I2S 1 L": the left half of DAC slot 1, which
//  says nothing about what plays there).  The slot name stays available for
//  a tooltip.  The dashboard pairs its output cards by the crossfeed pair
//  map when the device has one ("Woofers L" | "Woofers R").
//
//  The rules are the core's (`output_labels_for`, dspi-session
//  output_label.rs), over the view model's own state.  They run when the
//  names, the routing, the filters, the slot types or the pair map change,
//  coalesced, never per frame.  The macOS reference shows the stored name
//  (docs/UPSTREAM-DEVIATIONS.md, BUG1).
//

import Combine
import Foundation
import dspi_core

/// One dashboard card: two outputs side by side, or one (`right` nil).
struct OutputCardPlan: Hashable {
    let left: Int
    let right: Int?
    let leftName: String
    let rightName: String
}

extension DSPViewModel {
    /// What to show for output `output`: its label, else its stored name.
    func outputLabel(_ output: Int) -> String {
        if output >= 0, output < outputLabelTexts.count, !outputLabelTexts[output].isEmpty {
            return outputLabelTexts[output]
        }
        let ch = eqChannel(forOutput: output)
        return ch >= 0 && ch < channelNames.count ? channelNames[ch] : "Output \(output + 1)"
    }

    /// The firmware's name for output `output`'s slot ("I2S 1 L", "PDM"); "" before the labels.
    func outputSlotName(_ output: Int) -> String {
        output >= 0 && output < outputSlotNames.count ? outputSlotNames[output] : ""
    }

    /// The core's input: this view model's state, as the device holds it.
    func outputLabelInput() -> OutputLabelInput {
        let outputs = numOutputChannels
        func bands(_ eqCh: Int) -> [dspi_core.FilterParams] {
            ((channelData[eqCh] ?? []) + (xoverData[eqCh] ?? [])).map {
                dspi_core.FilterParams(filterType: UInt8(clamping: $0.type.rawValue), bypass: $0.bypass,
                                       freq: $0.freq, q: $0.q, gain: $0.gain, qp: $0.qp)
            }
        }
        return OutputLabelInput(numInputs: UInt8(clamping: chOut1),
                                numOutputs: UInt8(clamping: outputs),
                                outputTypes: Data(outputSlotTypes),
                                channelNames: channelNames,
                                matrixRouting: matrixRouting,
                                outputFilters: (0..<outputs).map { bands(eqChannel(forOutput: $0)) },
                                outputEnabled: outputEnabled,
                                crossfeedPairMap: Data(crossfeed.pairMap.bytes))
    }

    /// Read the labels and the cards again.  Publishes only what changed.
    func refreshOutputLabels() {
        let input = outputLabelInput()
        let labels = outputLabelsFor(input: input)
        let texts = labels.map(\.label)
        let slots = labels.map(\.slotName)
        let cards = outputCardsFor(input: input).map {
            OutputCardPlan(left: Int($0.left), right: $0.right.map(Int.init), leftName: $0.leftName, rightName: $0.rightName)
        }
        if texts != outputLabelTexts { outputLabelTexts = texts }
        if slots != outputSlotNames { outputSlotNames = slots }
        if cards != outputCardPlan { outputCardPlan = cards }
    }

    /// Follow the state the labels read: one refresh per burst of changes.
    func watchOutputLabels() {
        let changes: [AnyPublisher<Void, Never>] = [
            $channelNames.map { _ in () }.eraseToAnyPublisher(),
            $matrixRouting.map { _ in () }.eraseToAnyPublisher(),
            $outputEnabled.map { _ in () }.eraseToAnyPublisher(),
            $outputSlotTypes.map { _ in () }.eraseToAnyPublisher(),
            $channelData.map { _ in () }.eraseToAnyPublisher(),
            $xoverData.map { _ in () }.eraseToAnyPublisher(),
            crossfeed.$pairMap.map { _ in () }.eraseToAnyPublisher(),
        ]
        outputLabelWatch = Publishers.MergeMany(changes)
            .debounce(for: .milliseconds(50), scheduler: RunLoop.main)
            .sink { [weak self] in self?.refreshOutputLabels() }
    }
}
