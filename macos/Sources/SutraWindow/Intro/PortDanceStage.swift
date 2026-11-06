import Foundation

/// Shared floor projection and character rigs, in a 1,000-point composition.
enum PortDanceStage {
    static func project(_ point: CGPoint) -> CGPoint {
        let depth = (point.y - 462) / 485
        let perspective = 1 / (1 - 0.45 * depth)
        return CGPoint(x: 500 + (point.x - 500) * 0.72 * perspective,
                       y: 455 + depth * 340 * perspective)
    }
    static func scale(at point: CGPoint) -> CGFloat {
        0.72 / (1 - 0.45 * (point.y - 462) / 485)
    }
    static func palmContact(_ palm: CGPoint, angle: CGFloat, scale: CGFloat) -> CGPoint {
        let radians = angle * .pi / 180
        return CGPoint(x: palm.x - sin(radians) * 32 * scale,
                       y: palm.y + cos(radians) * 32 * scale)
    }

    struct Rig {
        let center: CGPoint
        let face: CGPoint
        let headAngle: CGFloat
        let headScale: CGFloat
        let shoulders: [CGPoint]
        let hips: [CGPoint]
        let armLengths: (CGFloat, CGFloat)
        let legLengths: (CGFloat, CGFloat)
        let width: CGFloat

        func root(_ limb: PortDanceChoreography.Limb) -> CGPoint {
            (limb.isHand ? shoulders : hips)[limb.isLeft ? 0 : 1]
        }
    }
    static func rig(_ robot: PortDanceChoreography.Robot) -> Rig {
        switch robot {
        case .blue:
            Rig(center: CGPoint(x: 245, y: 540), face: CGPoint(x: 240, y: 425), headAngle: -8, headScale: 0.9,
                shoulders: [CGPoint(x: 315, y: 500), CGPoint(x: 175, y: 500)],
                hips: [CGPoint(x: 270, y: 565), CGPoint(x: 215, y: 565)],
                armLengths: (130, 130), legLengths: (145, 145), width: 45)
        case .gold:
            Rig(center: CGPoint(x: 755, y: 540), face: CGPoint(x: 760, y: 425), headAngle: 8, headScale: 0.9,
                shoulders: [CGPoint(x: 825, y: 500), CGPoint(x: 685, y: 500)],
                hips: [CGPoint(x: 785, y: 565), CGPoint(x: 730, y: 565)],
                armLengths: (130, 130), legLengths: (145, 145), width: 45)
        case .rose:
            Rig(center: CGPoint(x: 500, y: 370), face: CGPoint(x: 500, y: 260), headAngle: -8, headScale: 0.76,
                shoulders: [CGPoint(x: 560, y: 345), CGPoint(x: 440, y: 345)],
                hips: [CGPoint(x: 530, y: 390), CGPoint(x: 470, y: 390)],
                armLengths: (115, 115), legLengths: (125, 125), width: 39)
        }
    }
    static func limbLengths(_ robot: PortDanceChoreography.Robot, _ limb: PortDanceChoreography.Limb) -> (CGFloat, CGFloat) {
        let rig = rig(robot)
        if limb.isHand { return rig.armLengths }
        if (robot == .blue && limb == .rightFoot) || (robot == .gold && limb == .leftFoot) { return (135, 135) }
        return rig.legLengths
    }
    struct BodyPose {
        let center: CGPoint
        let translation: CGPoint
        let angle: CGFloat
        let scale: CGFloat
        let turn: CGFloat
        func point(_ x: CGFloat, _ y: CGFloat) -> CGPoint {
            let radians = angle * .pi / 180
            let dx = (x - center.x) * scale
            let dy = (y - center.y) * scale
            return CGPoint(x: center.x + translation.x + dx * cos(radians) - dy * sin(radians),
                           y: center.y + translation.y + dx * sin(radians) + dy * cos(radians))
        }
        func point(_ p: CGPoint) -> CGPoint { point(p.x, p.y) }
    }

    /// Weight shifts before the limb leaves its dot. The winner stays committed through
    /// the hold; the other bodies unwind with their individual retreat timing.
    static func body(_ robot: PortDanceChoreography.Robot, frame: PortDanceChoreography.Frame, time: TimeInterval) -> BodyPose {
        let rig = rig(robot)
        let attempt = frame.attempt(robot)
        let target = project(frame.move.target)
        let dx = target.x - rig.center.x
        let dy = target.y - rig.center.y
        let length = max(1, hypot(dx, dy))
        let phase: Double = robot == .blue ? 0 : (robot == .gold ? 1.4 : 3)
        let amount = attempt.effort
        let distance: CGFloat = robot == .rose ? 132 : 125
        let travelX = max(-88, min(88, dx / length * distance))
        let travelY = min(robot == .rose ? 125 : 96, dy / length * distance)
        let isWinner = frame.move.robot == robot
        let recoil = frame.reaction(robot) * (isWinner ? 0 : 1)
        let impact = isWinner ? frame.impact : 0
        let translation = CGPoint(
            x: travelX * amount - dx / length * 10 * attempt.anticipation + sin(time * 1.4 + phase) * 1.7,
            y: travelY * amount - 9 * attempt.anticipation + impact * 12 + sin(time * 1.9 + phase) * 1.3
        )
        let lean = dx / length * (robot == .rose ? 12 : 16) * amount
            - dx / length * 3 * attempt.anticipation
            + (frame.rank(robot) == 1 ? -3 : 3) * recoil
        return BodyPose(center: rig.center, translation: translation,
                        angle: lean + sin(time * 1.4 + phase) * 0.5,
                        scale: 1 + translation.y * 0.00022 - impact * 0.018,
                        turn: dx / length * amount)
    }

    struct Point3 {
        var x: CGFloat
        var y: CGFloat
        var z: CGFloat
        static func + (a: Self, b: Self) -> Self { Self(x: a.x + b.x, y: a.y + b.y, z: a.z + b.z) }
        static func - (a: Self, b: Self) -> Self { Self(x: a.x - b.x, y: a.y - b.y, z: a.z - b.z) }
        static func * (a: Self, b: CGFloat) -> Self { Self(x: a.x * b, y: a.y * b, z: a.z * b) }
        func dot(_ b: Self) -> CGFloat { x * b.x + y * b.y + z * b.z }
        func cross(_ b: Self) -> Self {
            Self(x: y * b.z - z * b.y, y: z * b.x - x * b.z, z: x * b.y - y * b.x)
        }
        var length: CGFloat { sqrt(dot(self)) }
        var screen: CGPoint {
            let ground = project(CGPoint(x: x, y: y))
            return CGPoint(x: ground.x, y: ground.y - z * scale(at: CGPoint(x: x, y: y)))
        }
    }

    /// Recover a point above the same projective floor used by the circles. Depth allows
    /// a fixed-length bone to foreshorten naturally instead of folding flat against itself.
    static func unproject(_ point: CGPoint, height: CGFloat) -> Point3 {
        let screenY = point.y - 455
        let depth = (screenY + 0.72 * height) / (340 + 0.45 * screenY)
        let scale = 0.72 / (1 - 0.45 * depth)
        return Point3(x: 500 + (point.x - 500) / scale, y: 462 + depth * 485, z: height)
    }

    struct LimbPose {
        let root: Point3
        let joint: Point3
        let end: Point3
        let extensionRatio: CGFloat
    }

    static func articulation(_ robot: PortDanceChoreography.Robot, _ limb: PortDanceChoreography.Limb,
                             body: BodyPose, frame: PortDanceChoreography.Frame) -> LimbPose {
        let rig = rig(robot)
        let rootScreen = body.point(rig.root(limb))
        let contact = frame.position(.init(robot: robot, limb: limb))
        let lift = frame.elevation(robot, limb)
        let tipScale: CGFloat = robot == .rose ? 0.8 : 0.9
        let angle: CGFloat = limb.isHand ? (limb.isLeft ? -10 : 10) : (limb.isLeft ? 8 : -8)
        let offset: CGFloat = limb.isHand ? 49 : 26
        let radians = angle * .pi / 180
        let endScreen = CGPoint(x: contact.x + sin(radians) * offset * tipScale,
                                y: contact.y - cos(radians) * offset * tipScale)
        let ground = unproject(CGPoint(x: contact.x, y: contact.y + lift), height: 0)
        let floorScale = scale(at: CGPoint(x: ground.x, y: ground.y))
        let root = unproject(rootScreen, height: (limb.isHand ? 225 : (robot == .rose ? 165 : 180)) * body.scale)
        let end = unproject(endScreen, height: (lift + cos(radians) * offset * tipScale) / floorScale)
        let vector = end - root
        let distance = max(0.001, vector.length)
        let direction = vector * (1 / distance)
        let (upper, lower) = limbLengths(robot, limb)
        let along = (upper * upper - lower * lower + distance * distance) / (2 * distance)
        let height = sqrt(max(0, upper * upper - along * along))
        let side: CGFloat = limb.isLeft ? 1 : -1
        // A horizontal pole cannot align with a limb descending to the floor.
        // This avoids an undefined bend plane as a foot passes across the body.
        let pole = Point3(x: side, y: 0, z: 0)
        let perpendicular = pole - direction * pole.dot(direction)
        let bend = perpendicular * (1 / max(0.001, perpendicular.length))
        let tangent = direction.cross(bend)
        let middle = root + direction * along
        let preferred = middle + bend * height
        let screenDX = endScreen.x - rootScreen.x
        let screenDY = endScreen.y - rootScreen.y
        let screenLength = max(1, hypot(screenDX, screenDY))
        let halfway = CGPoint(x: (rootScreen.x + endScreen.x) / 2, y: (rootScreen.y + endScreen.y) / 2)
        let signedBend = (preferred.screen.x - halfway.x) * -screenDY + (preferred.screen.y - halfway.y) * screenDX
        let bow = min(42, screenLength * 0.28) * tanh(signedBend / (screenLength * 24))
        let desired = CGPoint(x: halfway.x - screenDY / screenLength * bow,
                              y: halfway.y + screenDX / screenLength * bow)
        func joint(_ swivel: CGFloat) -> Point3 {
            middle + (bend * cos(swivel) + tangent * sin(swivel)) * height
        }
        // Swivel the joint into depth, preserving both bone lengths. Favor an open
        // silhouette while staying near the anatomical bend and above the board.
        func score(_ swivel: CGFloat) -> CGFloat {
            let point = joint(swivel)
            let screen = point.screen
            let dx = screen.x - desired.x
            let dy = screen.y - desired.y
            return dx * dx + dy * dy + swivel * swivel * 80 + pow(max(0, 12 - point.z), 2) * 100
        }
        // Blend nearby swivel solutions instead of switching between two equal minima.
        // A hard minimum can pop a planted knee when the body begins to lean.
        let options = (-16...16).map { CGFloat($0) * .pi / 32 }
        let scores = options.map(score)
        let minimum = scores.min()!
        var total: CGFloat = 0
        var weighted: CGFloat = 0
        for (angle, value) in zip(options, scores) {
            let weight = exp(-(value - minimum) / 1200)
            total += weight
            weighted += angle * weight
        }
        return LimbPose(root: root, joint: joint(weighted / total), end: end,
                        extensionRatio: distance / (upper + lower))
    }
}
