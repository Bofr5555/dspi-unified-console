import XCTest
import Combine
@testable import DSPi_Console

/// BUG1: the core's output labels over the view model's state (Core/OutputLabels.swift), on the
/// K340's configuration (tests/fixtures/preset/k340-bug1.dspipreset): USB 1 feeds outputs 3-4,
/// USB 2 outputs 1-2; outputs 1 and 3 high cut, 2 and 4 low cut at 3.5 kHz; slots 1-2 I2S; the
/// one-DAC-per-ear pair map.
final class OutputLabelsBug1Tests: XCTestCase {
    private func k340() -> DSPViewModel {
        let vm = DSPViewModel(transport: FakeTransport(autoSelect: false))
        vm.outputSlotTypes = [1, 1, 0, 0]
        var names = vm.channelNames
        for (o, n) in ["I2S 1 L", "I2S 1 R", "I2S 2 L", "I2S 2 R"].enumerated() { names[vm.eqChannel(forOutput: o)] = n }
        vm.channelNames = names
        var routing = Array(repeating: Array(repeating: false, count: 9), count: vm.matrixRouting.count)
        routing[0][2] = true; routing[0][3] = true; routing[1][0] = true; routing[1][1] = true
        vm.matrixRouting = routing
        vm.outputEnabled = (0..<9).map { $0 < 4 }
        for o in 0..<4 {
            var bands = Array(repeating: FilterParams(), count: 10)
            bands[0] = FilterParams(type: o % 2 == 0 ? .lowPass : .highPass, freq: 3500, q: 0.707, gain: 0)
            vm.channelData[vm.eqChannel(forOutput: o)] = bands
        }
        vm.crossfeed.pairMap = .oneDACPerEar
        return vm
    }

    func testBUG1K340LabelsAndCards() {
        let vm = k340()
        vm.refreshOutputLabels()
        XCTAssertEqual(Array(vm.outputLabelTexts.prefix(4)),
                       ["Out 1 · R · Low", "Out 2 · R · High", "Out 3 · L · Low", "Out 4 · L · High"])
        XCTAssertEqual(vm.outputSlotName(0), "I2S 1 L")
        XCTAssertEqual(vm.outputLabel(3), "Out 4 · L · High")
        XCTAssertEqual(vm.outputCardPlan, [
            OutputCardPlan(left: 2, right: 0, leftName: "Woofers L", rightName: "Woofers R",
                           leftShortName: "Woofers L", rightShortName: "R"),
            OutputCardPlan(left: 3, right: 1, leftName: "Tweeters L", rightName: "Tweeters R",
                           leftShortName: "Tweeters L", rightShortName: "R"),
        ])
        // Review c86 F3: a phone's row beside its OUTn pill; macOS keeps the full label
        XCTAssertEqual((0..<4).map { vm.outputShortLabel($0) }, ["R · Low", "R · High", "L · Low", "L · High"])
        #if os(iOS)
        XCTAssertEqual(vm.outputRowLabel(1), "R · High")
        XCTAssertEqual(vm.outputCardPlan.first?.shownRight, "R")
        #else
        XCTAssertEqual(vm.outputRowLabel(1), "Out 2 · R · High")
        XCTAssertEqual(vm.outputCardPlan.first?.shownRight, "Woofers R")
        #endif
    }

    func testBUG1AUserNameWins() {
        let vm = k340()
        vm.channelNames[vm.eqChannel(forOutput: 0)] = "Right woofer"
        vm.refreshOutputLabels()
        XCTAssertEqual(vm.outputLabel(0), "Right woofer")
        XCTAssertEqual(vm.outputLabel(1), "Out 2 · R · High")
        XCTAssertEqual(vm.outputCardPlan.first?.rightName, "Right woofer")
        XCTAssertEqual(vm.outputShortLabel(0), "Right woofer")
    }

    /// Review c86 F1: outputs 1 and 2 low-passed and routed R and L: no "Woofers L" over the R one.
    func testBUG1CardSidesFollowTheRouting() {
        let vm = k340()
        vm.crossfeed.pairMap = .default
        var routing = Array(repeating: Array(repeating: false, count: 9), count: vm.matrixRouting.count)
        routing[1][0] = true; routing[0][1] = true
        vm.matrixRouting = routing
        var bands = Array(repeating: FilterParams(), count: 10)
        bands[0] = FilterParams(type: .lowPass, freq: 80, q: 0.707, gain: 0)
        vm.channelData[vm.eqChannel(forOutput: 1)] = bands
        vm.outputEnabled = (0..<9).map { $0 < 2 }
        vm.refreshOutputLabels()
        XCTAssertEqual(vm.outputCardPlan.first?.leftName, "Out 1 · R · Low")
        XCTAssertEqual(vm.outputCardPlan.first?.rightName, "Out 2 · L · Low")
    }

    /// The labels follow a change by themselves, once per burst (the debounced watch).
    func testBUG1LabelsFollowARename() {
        let vm = k340()
        let done = expectation(description: "the label follows")
        let watch = vm.$outputLabelTexts.dropFirst().sink { texts in
            if texts.first == "Sub" { done.fulfill() }
        }
        vm.channelNames[vm.eqChannel(forOutput: 0)] = "Sub"
        wait(for: [done], timeout: 2)
        watch.cancel()
    }
}
