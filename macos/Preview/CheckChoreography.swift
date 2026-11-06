import AppKit
import ImageIO
import UniformTypeIdentifiers

@main
struct CheckChoreography {
    static func main() {
        checkDepthSort()
        checkRaceTiming()
        let output = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "macos/.build/intro-preview/poses.png"
        let poseTime = CommandLine.arguments.count > 2 ? Double(CommandLine.arguments[2]) : nil
        let cg = CGContext(data: nil, width: 1920, height: 1280, bitsPerComponent: 8, bytesPerRow: 0,
                           space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
        cg.translateBy(x: 0, y: 1280)
        cg.scaleBy(x: 1, y: -1)
        cg.setFillColor(CGColor(srgbRed: 0.99, green: 0.96, blue: 0.93, alpha: 1))
        cg.fill(CGRect(x: 0, y: 0, width: 1920, height: 1280))
        let phases = [0.0, 0.85, 1.45, 2.22, 3.6, 4.3]
        for index in 0..<24 {
            cg.saveGState()
            cg.translateBy(x: CGFloat(index % 6) * 320, y: CGFloat(index / 6) * 320)
            let time = poseTime.map { Double(index) * 6 + $0 } ?? (Double(index / 6) * 6 + phases[index % 6])
            PortDanceDrawing.draw(in: cg, size: CGSize(width: 320, height: 320), time: time, showsCue: true)
            cg.restoreGState()
        }
        let destination = CGImageDestinationCreateWithURL(URL(fileURLWithPath: output) as CFURL, UTType.png.identifier as CFString, 1, nil)!
        CGImageDestinationAddImage(destination, cg.makeImage()!, nil)
        precondition(CGImageDestinationFinalize(destination))

        if CommandLine.arguments.contains("--poses-only") { return }
        typealias Dance = PortDanceChoreography
        var winningLimbs = Set<Dance.Contact>()
        var firstCalls = Set<String>()
        var performances = Set<String>()
        var largestExtension: CGFloat = 0
        var largestIdle: CGFloat = 0
        var smallestVisibleBend: CGFloat = 180
        for seed: UInt64 in 0..<100 {
            let first = Dance.frame(time: 0, seed: seed)
            firstCalls.insert(first.cue + first.move.robot.rawValue)
            var previousTarget: Dance.Port?
            var bag = Set<Dance.Port>()
            for index in 0..<(Dance.callsPerRound * 4) {
                let start = Double(index) * Dance.period
                let before = Dance.frame(time: start, seed: seed)
                winningLimbs.insert(before.move.contact)
                performances.insert(before.cue + before.move.robot.rawValue)
                precondition(before.move.destination != previousTarget, "Consecutive calls repeated the port")
                previousTarget = before.move.destination
                precondition(bag.insert(before.move.destination).inserted, "Repeated a port before exhausting the bag")
                if bag.count == Dance.targets.count {
                    precondition(bag == Set(Dance.targets))
                    bag.removeAll()
                }
                precondition(before.placements.count == 12 && Set(before.placements.values).count == 12)
                precondition(!before.placements.values.contains(before.move.destination), "Called an occupied dot")
                let target = PortDanceStage.project(before.move.target)
                let landed = Dance.frame(time: start + 2.15, seed: seed)
                near(landed.position(landed.move.contact), target)
                let end = Dance.frame(time: start + 5.99999, seed: seed)
                let next = Dance.frame(time: start + Dance.period, seed: seed)
                for contact in Dance.contacts { near(end.position(contact), next.position(contact)) }
                for phase in [0.0, 0.3, 0.55, 0.85, 1.1, 1.4, 1.55, 1.85, 2.22, 2.7, 3.2, 4.1, 4.65, 5.0, 5.4, 5.8, 5.99999] {
                    let frame = Dance.frame(time: start + phase, seed: seed)
                    var grounded: [CGPoint] = []
                    var onTarget = 0
                    for contact in Dance.contacts {
                        let point = frame.position(contact)
                        precondition(point.x.isFinite && point.y.isFinite)
                        if contact.limb != frame.move.limb {
                            near(point, PortDanceStage.project(frame.placements[contact]!.point))
                        }
                        if frame.elevation(contact.robot, contact.limb) < 0.001 {
                            for other in grounded { precondition(distance(point, other) > 1, "Two grounded limbs coincide") }
                            grounded.append(point)
                            if distance(point, target) < 0.001 { onTarget += 1 }
                        }
                        let body = PortDanceStage.body(contact.robot, frame: frame, time: start + phase)
                        let pose = PortDanceStage.articulation(contact.robot, contact.limb, body: body, frame: frame)
                        let lengths = PortDanceStage.limbLengths(contact.robot, contact.limb)
                        largestExtension = max(largestExtension, pose.extensionRatio)
                        precondition(pose.extensionRatio <= 1.001, "Unreachable \(contact) at \(phase): \(pose.extensionRatio)")
                        precondition(abs((pose.root - pose.joint).length - lengths.0) < 0.01, "Upper bone stretched: \(contact) at \(phase), extension \(pose.extensionRatio), actual \((pose.root - pose.joint).length), expected \(lengths.0)")
                        precondition(abs((pose.end - pose.joint).length - lengths.1) < 0.01, "Lower bone stretched")
                        precondition(pose.joint.z > 0, "Elbow or knee passed through the floor")
                        let upper = pose.root - pose.joint
                        let lower = pose.end - pose.joint
                        let spatialAngle = acos(min(1, max(-1, upper.dot(lower) / (upper.length * lower.length)))) * 180 / .pi
                        precondition(spatialAngle > 50, "Overfolded joint: \(contact), phase \(phase), angle \(spatialAngle)")
                        let a = CGPoint(x: pose.root.screen.x - pose.joint.screen.x, y: pose.root.screen.y - pose.joint.screen.y)
                        let b = CGPoint(x: pose.end.screen.x - pose.joint.screen.x, y: pose.end.screen.y - pose.joint.screen.y)
                        if hypot(a.x, a.y) > 15 && hypot(b.x, b.y) > 15 {
                            let cosine = (a.x * b.x + a.y * b.y) / (hypot(a.x, a.y) * hypot(b.x, b.y))
                            let angle = acos(min(1, max(-1, cosine))) * 180 / .pi
                            // The rear knees are viewed almost along their bones and sit behind
                            // the foreground heads; use spatial angles for those foreshortened joints.
                            if contact.robot != .rose {
                                smallestVisibleBend = min(smallestVisibleBend, angle)
                                precondition(angle > 45, "Limb folded back: \(contact), phase \(phase), angle \(angle)")
                            }
                        }

                    }
                    precondition(onTarget == (phase >= Dance.landingTime && phase <= Dance.resetStart ? 1 : 0))
                    for robot in Dance.Robot.allCases {
                        let body = PortDanceStage.body(robot, frame: frame, time: start + phase)
                        let face = body.point(PortDanceStage.rig(robot).face)
                        // Keep the called circle below every face, including the forward lunge.
                        precondition(face.y + 125 < target.y - 35, "Face hid the called circle")
                        precondition(body.translation.x.isFinite && body.translation.y.isFinite && body.angle.isFinite)
                    }
                }
                for robot in Dance.Robot.allCases {
                    let contact = Dance.Contact(robot: robot, limb: before.move.limb)
                    let source = PortDanceStage.project(before.placements[contact]!.point)
                    let reaching = Dance.frame(time: start + 1.5, seed: seed)
                    var point = reaching.position(contact)
                    point.y += reaching.elevation(robot, contact.limb)
                    let dx = target.x - source.x
                    let dy = target.y - source.y
                    precondition((point.x - source.x) * dx + (point.y - source.y) * dy > 0)
                    precondition(abs((point.x - source.x) * dy - (point.y - source.y) * dx) < 0.001)
                    if robot != before.move.robot {
                        precondition(before.arrival(robot) - Dance.landingTime >= 0.44, "Winner's landing has no readable lead")
                        let closest = Dance.frame(time: start + before.arrival(robot), seed: seed)
                        var hover = closest.position(contact)
                        hover.y += closest.elevation(robot, contact.limb)
                        precondition(distance(hover, target) < 22.01 && closest.elevation(robot, contact.limb) >= 15)
                        near(Dance.frame(time: start + Dance.recoveryTime, seed: seed).position(contact), source)
                    }
                    let idle = Dance.frame(time: start + 5.8, seed: seed)
                    let idleBody = PortDanceStage.body(robot, frame: idle, time: start + 5.8)
                    let endBody = PortDanceStage.body(robot, frame: end, time: start + 5.99999)
                    let nextBody = PortDanceStage.body(robot, frame: next, time: start + Dance.period)
                    largestIdle = max(largestIdle, distance(idleBody.translation, endBody.translation))
                    precondition(distance(idleBody.translation, endBody.translation) < 2)
                    precondition(distance(endBody.translation, nextBody.translation) < 0.001)
                }
            }
        }
        // Densely sample each gesture: IK must not flip at a fully folded joint.
        var maximumJointStep: CGFloat = 0
        for index in 0..<Dance.callsPerRound {
            var previous: [Dance.Contact: CGPoint] = [:]
            for tick in 0...360 {
                let time = Double(index) * Dance.period + Double(tick) / 60
                let frame = Dance.frame(time: time, seed: Dance.exportSeed)
                for contact in Dance.contacts {
                    let body = PortDanceStage.body(contact.robot, frame: frame, time: time)
                    let pose = PortDanceStage.articulation(contact.robot, contact.limb, body: body, frame: frame)
                    let joint = pose.joint.screen

                    if let last = previous[contact] {
                        let step = distance(last, joint)
                        maximumJointStep = max(maximumJointStep, step)
                        precondition(step < 24, "Joint snapped: \(contact), time \(time), step \(step)")
                    }
                    previous[contact] = joint
                }
            }
        }
        print("Largest elbow/knee step at 60 fps: \(maximumJointStep)")
        precondition(winningLimbs.count == 12 && firstCalls.count >= 10 && performances.count == 16)
        print("Verified 9,600 staged races: four varied, nonrepeating free targets, fixed limb lengths, one landing, staggered near misses, visible targets, reserved supports, and continuous resets.")
        print("Smallest visible elbow/knee angle: \(smallestVisibleBend)")
        print("Maximum limb extension: \(largestExtension); maximum idle travel: \(largestIdle)")
    }

    static func checkRaceTiming() {
        typealias Dance = PortDanceChoreography
        // A race should cover ground early, while still leaving and planting smoothly.
        for index in 0..<Dance.callsPerRound {
            let start = Double(index) * Dance.period
            let frame = Dance.frame(time: start, seed: Dance.exportSeed)
            let contact = frame.move.contact
            let source = frame.position(contact)
            let target = PortDanceStage.project(frame.move.target)
            let span = Dance.landingTime - Dance.raceStart
            let quarter = Dance.frame(time: start + Dance.raceStart + span / 4, seed: Dance.exportSeed)
            let half = Dance.frame(time: start + Dance.raceStart + span / 2, seed: Dance.exportSeed)
            precondition(quarter.attempt(contact.robot).reach > 0.35, "Launch feels delayed")
            precondition(half.attempt(contact.robot).reach > 0.8, "Reach never gets up to speed")
            let epsilon = 0.00001
            let departing = Dance.frame(time: start + Dance.raceStart + epsilon, seed: Dance.exportSeed)
            let arriving = Dance.frame(time: start + Dance.landingTime - epsilon, seed: Dance.exportSeed)
            precondition(distance(departing.position(contact), source) / epsilon < 1, "Abrupt takeoff")
            precondition(distance(arriving.position(contact), target) / epsilon < 1, "Abrupt landing")
        }
        print("Verified quick launches and smooth takeoff/landing endpoints across the full port/limb repertoire.")
    }

    /// A single limb must be behind the body at one end and in front at the other,
    /// regardless of submission order. Check the actual composited pixels, not just sort keys.
    static func checkDepthSort() {
        for reverse in [false, true] {
            for bodyFirst in [false, true] {
                let cg = CGContext(data: nil, width: 200, height: 100, bitsPerComponent: 8, bytesPerRow: 800,
                                   space: CGColorSpace(name: CGColorSpace.sRGB)!,
                                   bitmapInfo: CGBitmapInfo.byteOrder32Big.rawValue | CGImageAlphaInfo.premultipliedLast.rawValue)!
                let layers = PortDanceLayers()
                func body() {
                    layers.add(depth: 50) { context in
                        context.setFillColor(CGColor(srgbRed: 0, green: 1, blue: 0, alpha: 1))
                        context.fill(CGRect(x: 40, y: 30, width: 120, height: 40))
                    }
                }
                if bodyFirst { body() }
                layers.addSegment(from: CGPoint(x: 20, y: 50), to: CGPoint(x: 180, y: 50),
                                  startDepth: reverse ? 100 : 0, endDepth: reverse ? 0 : 100, width: 10) { context in
                    context.setFillColor(CGColor(srgbRed: 1, green: 0, blue: 0, alpha: 1))
                    context.fill(CGRect(x: 20, y: 45, width: 160, height: 10))
                }
                if !bodyFirst { body() }
                layers.render(in: cg)
                let bytes = cg.data!.assumingMemoryBound(to: UInt8.self)
                for (x, red) in [(60, reverse), (140, !reverse)] {
                    let offset = 50 * 800 + x * 4
                    precondition(bytes[offset + (red ? 0 : 1)] == 255, "Incorrect limb/body occlusion")
                    precondition(bytes[offset + (red ? 1 : 0)] == 0, "Depth slice leaked over an occluder")
                }
                for x in 22..<179 {
                    precondition(bytes[50 * 800 + x * 4 + 3] == 255, "A seam opened between depth slices")
                }
            }
        }
        let cg = CGContext(data: nil, width: 200, height: 200, bitsPerComponent: 8, bytesPerRow: 800,
                           space: CGColorSpace(name: CGColorSpace.sRGB)!,
                           bitmapInfo: CGBitmapInfo.byteOrder32Big.rawValue | CGImageAlphaInfo.premultipliedLast.rawValue)!
        let layers = PortDanceLayers()
        layers.addSegment(from: CGPoint(x: 100, y: 10), to: CGPoint(x: 100, y: 190),
                          startDepth: 0, endDepth: 100, width: 40) { context in
            context.setStrokeColor(CGColor(srgbRed: 0, green: 0, blue: 1, alpha: 1))
            context.setLineWidth(40)
            context.move(to: CGPoint(x: 100, y: 10))
            context.addLine(to: CGPoint(x: 100, y: 190))
            context.strokePath()
        }
        layers.addSegment(from: CGPoint(x: 20, y: 170), to: CGPoint(x: 180, y: 30),
                          startDepth: 60, endDepth: 60, width: 40) { context in
            context.setStrokeColor(CGColor(srgbRed: 1, green: 1, blue: 1, alpha: 1))
            context.setLineWidth(40)
            context.move(to: CGPoint(x: 20, y: 170))
            context.addLine(to: CGPoint(x: 180, y: 30))
            context.strokePath()
        }
        layers.render(in: cg)
        let bytes = cg.data!.assumingMemoryBound(to: UInt8.self)
        for y in 78...122 {
            precondition(bytes[y * 800 + 100 * 4] == 255, "Rear limb cut through crossing at y=\(y), red=\(bytes[y * 800 + 100 * 4])")
        }
        let folded = CGContext(data: nil, width: 200, height: 200, bitsPerComponent: 8, bytesPerRow: 800,
                               space: CGColorSpace(name: CGColorSpace.sRGB)!,
                               bitmapInfo: CGBitmapInfo.byteOrder32Big.rawValue | CGImageAlphaInfo.premultipliedLast.rawValue)!
        let foldedLayers = PortDanceLayers()
        for (start, end, nearDepth, farDepth, color) in [
            (CGPoint(x: 60, y: 180), CGPoint(x: 100, y: 30), CGFloat(0), CGFloat(100), CGColor(srgbRed: 1, green: 0, blue: 0, alpha: 1)),
            (CGPoint(x: 100, y: 30), CGPoint(x: 120, y: 180), CGFloat(100), CGFloat(0), CGColor(srgbRed: 0, green: 0, blue: 1, alpha: 1)),
        ] {
            foldedLayers.addSegment(from: start, to: end, startDepth: nearDepth, endDepth: farDepth, width: 40) { context in
                context.setStrokeColor(color)
                context.setLineWidth(40)
                context.move(to: start)
                context.addLine(to: end)
                context.strokePath()
            }
        }
        foldedLayers.render(in: folded)
        let foldedBytes = folded.data!.assumingMemoryBound(to: UInt8.self)
        for y in 45...105 {
            precondition(foldedBytes[y * 800 + 100 * 4 + 2] == 255, "Folded upper arm nicked the forearm")
        }
        print("Verified limb/body crossings in both directions and drawing orders, seamless slices, clean limb/limb overlaps, and folded elbows.")
    }

    static func distance(_ a: CGPoint, _ b: CGPoint) -> CGFloat { hypot(a.x - b.x, a.y - b.y) }
    static func near(_ a: CGPoint, _ b: CGPoint) { precondition(distance(a, b) < 0.001) }
}
