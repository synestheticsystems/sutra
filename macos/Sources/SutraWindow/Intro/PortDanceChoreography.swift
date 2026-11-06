import Foundation

/// A small repertoire of staged races. Each performance ends with a visible reset,
/// keeping the next call's target free without accumulating tangled poses.
enum PortDanceChoreography {
    enum Robot: String, CaseIterable { case blue = "Blue", gold = "Gold", rose = "Pearl" }
    enum Limb: String, CaseIterable {
        case leftHand = "Left hand", rightHand = "Right hand", leftFoot = "Left foot", rightFoot = "Right foot"
        var isLeft: Bool { self == .leftHand || self == .leftFoot }
        var isHand: Bool { self == .leftHand || self == .rightHand }
    }
    struct Contact: Hashable { let robot: Robot; let limb: Limb }

    static let labels = [
        ["8081", "8082", "3003", "5432", "9090", "80"],
        ["8080", "4200", "3002", "5000", "443", "8888"],
        ["22", "9000", "3000", "3001", "8001", "8000"],
        ["8880", "4000", "6006", "5173", "9001", "8443"],
    ]
    struct Port: Hashable {
        let index: Int
        var name: String { labels[index / 6][index % 6] }
        var point: CGPoint { CGPoint(x: 125 + 150 * (index % 6), y: 540 + 115 * (index / 6)) }
    }
    struct Move {
        let contact: Contact
        let destination: Port
        var robot: Robot { contact.robot }
        var limb: Limb { contact.limb }
        var port: String { destination.name }
        var target: CGPoint { destination.point }
    }
    enum Outcome { case racing, won, lost }
    struct Attempt {
        let reach: CGFloat
        let lift: CGFloat
        let effort: CGFloat
        let anticipation: CGFloat
    }
    struct Frame {
        let move: Move
        let beat: TimeInterval
        var placements: [Contact: Port] { initialPlacements }
        var cue: String { "\(move.limb.rawValue), \(move.port)!" }
        var cueOpacity: CGFloat { 1 - smooth((beat - 4.5) / 0.45) }
        var reset: CGFloat { smooth((beat - resetStart) / (resetEnd - resetStart)) }
        var settle: CGFloat { smooth((beat - landingTime) / 0.35) }

        /// The two late arrivals have distinct timing: an overreach, then a hesitation.
        func rank(_ robot: Robot) -> Int {
            if robot == move.robot { return 0 }
            let order = Robot.allCases
            let winner = order.firstIndex(of: move.robot)!
            return (order.firstIndex(of: robot)! - winner + order.count) % order.count
        }
        func arrival(_ robot: Robot) -> Double {
            [landingTime, 1.85, 2.22][rank(robot)]
        }
        func outcome(_ robot: Robot) -> Outcome {
            beat < (robot == move.robot ? landingTime : arrival(robot) - 0.12)
                ? .racing : (robot == move.robot ? .won : .lost)
        }
        func reaction(_ robot: Robot) -> CGFloat {
            smooth((beat - (robot == move.robot ? landingTime + 0.12 : arrival(robot) - 0.12)) / 0.2)
                * (1 - smooth((beat - 4.25) / 0.4))
        }
        /// A short compression on contact gives the winning plant weight.
        var impact: CGFloat {
            let phase = (beat - landingTime) / 0.34
            return phase > 0 && phase < 1 ? sin(smooth(phase) * .pi) : 0
        }
        func attempt(_ robot: Robot) -> Attempt {
            let rank = rank(robot)
            let liftScale: CGFloat = move.limb.isHand ? 1 : (robot == .rose ? 0.45 : 0.65)
            let start = raceStart + [0.0, 0.12, 0.3][rank]
            let arrival = arrival(robot)
            let progress = min(1, max(0, (beat - start) / (arrival - start)))
            let anticipation = smooth(beat / 0.28) * (1 - smooth((beat - start + 0.18) / 0.25))
            let outward = launch(progress)
            if rank == 0 {
                let reach = outward * (1 - reset)
                let lift = sin(outward * .pi) * (move.limb.isHand ? 32 : 55)
                    + sin(reset * .pi) * 32
                // Keep weight over the planted limb until the winner steps home.
                let effort = launch((beat - start + 0.06) / (arrival - start + 0.06))
                    * (1 - settle * 0.14) * (1 - reset)
                return Attempt(reach: reach, lift: lift * liftScale, effort: effort, anticipation: anticipation)
            }
            let retreatStart = arrival + (rank == 1 ? 0.22 : 0.3)
            let retreatEnd = rank == 1 ? 3.6 : 4.05
            let retreat = smooth((beat - retreatStart) / (retreatEnd - retreatStart))
            let source = PortDanceStage.project(placements[Contact(robot: robot, limb: move.limb)]!.point)
            let target = PortDanceStage.project(move.target)
            let distance = hypot(target.x - source.x, target.y - source.y)
            let reach = (1 - 22 / max(30, distance)) * outward * (1 - retreat)
            let lift = sin(outward * .pi) * (move.limb.isHand ? 28 : 45) + outward * (1 - retreat) * (rank == 1 ? 34 : 55)
                + sin(retreat * .pi) * (rank == 1 ? 46 : 20)
            return Attempt(reach: reach, lift: lift * liftScale,
                           effort: launch((beat - start + 0.06) / (arrival - start + 0.06)) * (1 - retreat),
                           anticipation: anticipation)
        }
        func position(_ contact: Contact) -> CGPoint {
            let source = PortDanceStage.project(placements[contact]!.point)
            guard contact.limb == move.limb else { return source }
            let attempt = attempt(contact.robot)
            let destination = PortDanceStage.project(move.target)
            return CGPoint(x: source.x + (destination.x - source.x) * attempt.reach,
                           y: source.y + (destination.y - source.y) * attempt.reach - attempt.lift)
        }
        func elevation(_ robot: Robot, _ limb: Limb) -> CGFloat {
            move.limb == limb ? attempt(robot).lift : 0
        }
    }

    static let period = 6.0
    static let raceStart = 0.55
    static let landingTime = 1.4
    static let recoveryTime = 4.05
    static let resetStart = 4.65
    static let resetEnd = 5.75
    static let exportSeed: UInt64 = 33
    static let contacts = Robot.allCases.flatMap { robot in Limb.allCases.map { Contact(robot: robot, limb: $0) } }
    static let targets = [14, 15, 20, 21].map { Port(index: $0) }
    // Six visits cover both limbs and all three winners on each yellow target.
    static let callsPerRound = targets.count * 6

    // Four central yellow/green ports stay open. Every support has its own dot around them.
    static let initialPlacements: [Contact: Port] = Dictionary(uniqueKeysWithValues: contacts.map { contact in
        let index: Int
        switch (contact.robot, contact.limb) {
        case (.blue, .leftHand): index = 13
        case (.blue, .rightHand): index = 12
        case (.blue, .leftFoot): index = 19
        case (.blue, .rightFoot): index = 18
        case (.gold, .leftHand): index = 17
        case (.gold, .rightHand): index = 16
        case (.gold, .leftFoot): index = 23
        case (.gold, .rightFoot): index = 22
        case (.rose, .leftHand): index = 9
        case (.rose, .rightHand): index = 8
        case (.rose, .leftFoot): index = 3
        case (.rose, .rightFoot): index = 2
        }
        return (contact, Port(index: index))
    })

    static func frame(time: TimeInterval, seed: UInt64) -> Frame {
        let time = max(0, time.isFinite ? time : 0)
        let slot = Int((time / period).truncatingRemainder(dividingBy: 1_000_000))
        let batch = slot / targets.count
        var destinations = targetOrder(batch: batch, seed: seed)
        // Swapping only the first two preserves the previous bag's final element,
        // so the boundary can be checked directly without replaying earlier frames.
        if batch > 0, destinations[0] == targetOrder(batch: batch - 1, seed: seed).last {
            destinations.swapAt(0, 1)
        }
        let destination = destinations[slot % targets.count]
        var random = Shuffle(seed: seed ^ (UInt64(destination.index) &* 0x9e3779b97f4a7c15)
                             ^ UInt64(batch / 6))
        let contenders = random.shuffled(contenders(for: destination))
        let contact = contenders[batch % contenders.count]
        return Frame(move: Move(contact: contact, destination: destination),
                     beat: time.truncatingRemainder(dividingBy: period))
    }

    /// Each bag calls every open target once, in a fresh order.
    private static func targetOrder(batch: Int, seed: UInt64) -> [Port] {
        var random = Shuffle(seed: seed ^ 0xa0761d6478bd642f ^ UInt64(batch))
        return random.shuffled(targets)
    }

    private static func contenders(for destination: Port) -> [Contact] {
        // The front row needs a foot reach. Its nearer robots can plant while
        // the rear robot still makes the same directed, airborne losing attempt.
        contacts.filter { contact in
            switch destination.index {
            case 14: !contact.limb.isLeft
            case 15: contact.limb.isLeft
            case 20: contact.limb == .rightFoot && contact.robot != .rose
            case 21: contact.limb == .leftFoot && contact.robot != .rose
            default: false
            }
        }
    }

    /// A brief acceleration, then a longer braking phase. Peak speed arrives one
    /// quarter into the reach; both endpoints still have zero velocity.
    private static func launch(_ value: Double) -> Double {
        let t = min(1, max(0, value))
        return t * t * (10 + t * (-20 + t * (15 - 4 * t)))
    }

    private static func smooth(_ value: Double) -> Double {
        let t = min(1, max(0, value))
        return t * t * t * (10 + t * (-15 + 6 * t))
    }
}

private struct Shuffle {
    var seed: UInt64
    mutating func shuffled<Element>(_ values: [Element]) -> [Element] {
        var result = values
        for index in result.indices.dropFirst().reversed() {
            result.swapAt(index, Int(next() % UInt64(index + 1)))
        }
        return result
    }

    mutating func next() -> UInt64 {
        seed &+= 0x9e3779b97f4a7c15
        var value = seed
        value = (value ^ (value >> 30)) &* 0xbf58476d1ce4e5b9
        value = (value ^ (value >> 27)) &* 0x94d049bb133111eb
        return value ^ (value >> 31)
    }
}
