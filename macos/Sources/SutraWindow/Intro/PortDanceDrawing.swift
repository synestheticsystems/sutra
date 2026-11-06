import AppKit

/// Vector artwork shared by the animated canvas and the PNG exporter; coordinates are in a
/// 1,000-point square, with the origin at the top left. Hands and feet stay planted as bodies sway.
enum PortDanceDrawing {
    static func draw(in context: CGContext, size: CGSize, time: TimeInterval, seed: UInt64 = PortDanceChoreography.exportSeed, showsCue: Bool = false) {
        context.saveGState()
        defer { context.restoreGState() }
        context.clip(to: CGRect(origin: .zero, size: size))
        let scale = min(size.width, size.height) / 1_000
        context.translateBy(x: (size.width - 1_000 * scale) / 2, y: (size.height - 1_000 * scale) / 2)
        context.scaleBy(x: scale, y: scale)
        let frame = PortDanceChoreography.frame(time: time, seed: seed)
        let painter = Painter(context, frame: frame, showsCue: showsCue, time: time)
        if showsCue { painter.cue() }
        painter.mat(time: time)
        if showsCue {
            painter.targetResult()
        }
        for robot in PortDanceChoreography.Robot.allCases { painter.robot(robot, time: time) }
        painter.layers.render(in: context)
    }
}

private struct Enamel {
    let light: CGColor
    let mid: CGColor
    let dark: CGColor
    let glow: CGColor

    static let blue = Enamel(light: rgb(0x9eeaff), mid: rgb(0x16b5ff), dark: rgb(0x0069bf), glow: rgb(0x00fff0))
    static let gold = Enamel(light: rgb(0xffed9b), mid: rgb(0xffbc00), dark: rgb(0xe87b00), glow: rgb(0xf6ff54))
    static let rose = Enamel(light: rgb(0xfffdfb), mid: rgb(0xf5dedb), dark: rgb(0xc69fa9), glow: rgb(0x00fff0))
}

private func rgb(_ hex: UInt32, alpha: CGFloat = 1) -> CGColor {
    CGColor(srgbRed: CGFloat((hex >> 16) & 255) / 255, green: CGFloat((hex >> 8) & 255) / 255,
            blue: CGFloat(hex & 255) / 255, alpha: alpha)
}

private struct Painter {
    let cg: CGContext
    let frame: PortDanceChoreography.Frame
    let showsCue: Bool
    let bodies: [PortDanceChoreography.Robot: PortDanceStage.BodyPose]
    let layers = PortDanceLayers()
    let ink = rgb(0x141b22)
    let white = rgb(0xffffff)

    init(_ context: CGContext, frame: PortDanceChoreography.Frame, showsCue: Bool, time: TimeInterval) {
        cg = context
        self.frame = frame
        self.showsCue = showsCue
        bodies = Dictionary(uniqueKeysWithValues: PortDanceChoreography.Robot.allCases.map {
            ($0, PortDanceStage.body($0, frame: frame, time: time))
        })
    }

    func enqueue(depth: CGFloat, _ draw: @escaping () -> Void) {
        layers.add(depth: depth) { _ in draw() }
    }

    func bodyDepth(_ robot: PortDanceChoreography.Robot) -> CGFloat {
        let body = bodies[robot]!
        return PortDanceStage.unproject(body.point(body.center), height: 205 * body.scale).y
    }

    func contactShadow(_ point: CGPoint, elevation: CGFloat, width: CGFloat, height: CGFloat) {
        enqueue(depth: -10_000 + point.y) {
            saved {
                cg.setAlpha(max(0.25, 1 - elevation / 140))
                shadow(x: point.x, y: point.y + elevation, width: width, height: height)
            }
        }
    }

    func saved(_ body: () -> Void) {
        cg.saveGState()
        body()
        cg.restoreGState()
    }

    func at(_ x: CGFloat, _ y: CGFloat, angle: CGFloat = 0, scale: CGFloat = 1, _ body: () -> Void) {
        saved {
            cg.translateBy(x: x, y: y)
            cg.rotate(by: angle * .pi / 180)
            cg.scaleBy(x: scale, y: scale)
            body()
        }
    }

    func ellipse(_ x: CGFloat, _ y: CGFloat, _ w: CGFloat, _ h: CGFloat) -> CGPath {
        CGPath(ellipseIn: CGRect(x: x, y: y, width: w, height: h), transform: nil)
    }

    func rounded(_ x: CGFloat, _ y: CGFloat, _ w: CGFloat, _ h: CGFloat, _ r: CGFloat) -> CGPath {
        CGPath(roundedRect: CGRect(x: x, y: y, width: w, height: h), cornerWidth: r, cornerHeight: r, transform: nil)
    }

    func path(_ body: (CGMutablePath) -> Void) -> CGPath {
        let p = CGMutablePath()
        body(p)
        return p
    }

    func fill(_ path: CGPath, _ color: CGColor) {
        cg.addPath(path)
        cg.setFillColor(color)
        cg.fillPath()
    }

    func stroke(_ path: CGPath, _ color: CGColor, width: CGFloat) {
        cg.addPath(path)
        cg.setStrokeColor(color)
        cg.setLineWidth(width)
        cg.setLineJoin(.round)
        cg.setLineCap(.round)
        cg.strokePath()
    }

    func gradient(_ shape: CGPath, colors: [CGColor], from: CGPoint? = nil, to: CGPoint? = nil, border: CGFloat = 5) {
        let box = shape.boundingBox
        saved {
            cg.addPath(shape)
            cg.clip()
            let gradient = CGGradient(colorsSpace: CGColorSpace(name: CGColorSpace.sRGB)!, colors: colors as CFArray, locations: nil)!
            cg.drawLinearGradient(gradient,
                                  start: from ?? CGPoint(x: box.minX, y: box.minY),
                                  end: to ?? CGPoint(x: box.maxX * 0.35 + box.minX * 0.65, y: box.maxY),
                                  options: [.drawsBeforeStartLocation, .drawsAfterEndLocation])
        }
        if border > 0 { stroke(shape, ink, width: border) }
    }

    func enamel(_ shape: CGPath, _ paint: Enamel, border: CGFloat = 5) {
        gradient(shape, colors: [paint.light, paint.mid, paint.dark], border: border)
    }

    func shadow(x: CGFloat, y: CGFloat, width: CGFloat, height: CGFloat) {
        saved {
            cg.setShadow(offset: CGSize(width: 0, height: 5), blur: 12, color: rgb(0x382634, alpha: 0.23))
            fill(ellipse(x - width / 2, y - height / 2, width, height), rgb(0x382634, alpha: 0.11))
        }
    }

    func line(_ a: CGPoint, _ b: CGPoint) -> CGPath {
        path { p in p.move(to: a); p.addLine(to: b) }
    }

    func label(_ text: String, x: CGFloat, y: CGFloat, color: CGColor) {
        saved {
            cg.translateBy(x: x, y: y)
            cg.scaleBy(x: 1, y: -1)
            let font = CTFontCreateWithName("HelveticaNeue-BoldItalic" as CFString, 29, nil)
            let string = NSAttributedString(string: text, attributes: [
                NSAttributedString.Key(kCTFontAttributeName as String): font,
                NSAttributedString.Key(kCTForegroundColorAttributeName as String): color,
            ])
            let textLine = CTLineCreateWithAttributedString(string)
            let width = CTLineGetTypographicBounds(textLine, nil, nil, nil)
            cg.textPosition = CGPoint(x: -width / 2, y: -10)
            CTLineDraw(textLine, cg)
        }
    }

    func cue() {
        saved {
            cg.setAlpha(frame.cueOpacity)
            fill(rounded(250, 38, 500, 62, 31), rgb(0xffffff, alpha: 0.94))
            stroke(rounded(250, 38, 500, 62, 31), rgb(0xd9d2cc), width: 1.5)
            label(frame.cue, x: 500, y: 68, color: ink)
        }
    }

    func targetResult() {
        let target = frame.move.target
        let won = frame.beat >= PortDanceChoreography.landingTime
        saved {
            cg.setAlpha(frame.cueOpacity)
            let ring = projected(ellipse(target.x - 76, target.y - 38, 152, 76))
            stroke(ring, rgb(0xffffff, alpha: 0.96), width: 10)
            stroke(ring, ink.copy(alpha: won ? 0.7 : 0.45)!, width: 2.5)
            // A brief expanding floor ripple makes the actual contact the punctuation.
            let elapsed = frame.beat - PortDanceChoreography.landingTime
            if elapsed >= 0 && elapsed < 0.5 {
                let spread = elapsed * 48
                let ripple = projected(ellipse(target.x - 76 - spread, target.y - 38 - spread / 2,
                                              152 + spread * 2, 76 + spread))
                stroke(ripple, rgb(0xffffff, alpha: 1 - elapsed * 2), width: 6)
            }
        }
    }

    func projected(_ original: CGPath) -> CGPath {
        let result = CGMutablePath()
        original.applyWithBlock { element in
            let points = element.pointee.points
            switch element.pointee.type {
            case .moveToPoint: result.move(to: PortDanceStage.project(points[0]))
            case .addLineToPoint: result.addLine(to: PortDanceStage.project(points[0]))
            case .addQuadCurveToPoint:
                result.addQuadCurve(to: PortDanceStage.project(points[1]), control: PortDanceStage.project(points[0]))
            case .addCurveToPoint:
                result.addCurve(to: PortDanceStage.project(points[2]), control1: PortDanceStage.project(points[0]), control2: PortDanceStage.project(points[1]))
            case .closeSubpath: result.closeSubpath()
            @unknown default: break
            }
        }
        return result
    }

    func mat(time: TimeInterval) {
        let board = projected(rounded(0, 462, 1_000, 485, 12))
        saved {
            cg.setShadow(offset: CGSize(width: 0, height: 9), blur: 20, color: rgb(0x372334, alpha: 0.13))
            fill(board, rgb(0xfff7f0))
        }
        gradient(board, colors: [rgb(0xfffcf7), rgb(0xfaece5)], border: 3)
        let colors: [(UInt32, UInt32)] = [(0xff5558, 0xf51532), (0x14b9ff, 0x0077eb), (0xffe94b, 0xf6c400), (0x6cdd28, 0x29b200)]
        let ports = PortDanceChoreography.labels
        for row in 0..<4 {
            for col in 0..<6 {
                let x = CGFloat(125 + 150 * col)
                let y = CGFloat(540 + 115 * row)
                let dot = projected(ellipse(x - 68, y - 29, 136, 58))
                gradient(dot, colors: [rgb(colors[row].0), rgb(colors[row].1)], border: 0)
                if !ports[row][col].isEmpty {
                    let point = PortDanceStage.project(CGPoint(x: x, y: y + 3))
                    at(point.x, point.y, scale: PortDanceStage.scale(at: CGPoint(x: x, y: y))) {
                        cg.scaleBy(x: 1, y: 0.75)
                        label(ports[row][col], x: 0, y: 0,
                              color: row == 2 ? rgb(0x776400, alpha: 0.8) : rgb(0xffffff, alpha: 0.65))
                    }
                }
            }
        }
    }

    func joint(_ point: CGPoint, radius: CGFloat = 21) {
        gradient(ellipse(point.x - radius, point.y - radius, radius * 2, radius * 2),
                 colors: [rgb(0xe9dfe1), rgb(0x625760), rgb(0x27232d)])
        fill(ellipse(point.x - radius * 0.45, point.y - radius * 0.55, radius * 0.5, radius * 0.25), rgb(0xffffff, alpha: 0.45))
    }

    func segment(_ start: CGPoint, _ end: CGPoint, width: CGFloat, paint: Enamel) {
        let dx = end.x - start.x
        let dy = end.y - start.y
        let length = hypot(dx, dy)
        at(start.x, start.y, angle: atan2(dy, dx) * 180 / .pi - 90) {
            let shape = path { p in
                p.move(to: CGPoint(x: -width * 0.42, y: 2))
                p.addCurve(to: CGPoint(x: -width * 0.38, y: length - 2), control1: CGPoint(x: -width * 0.64, y: length * 0.3), control2: CGPoint(x: -width * 0.43, y: length * 0.72))
                p.addQuadCurve(to: CGPoint(x: width * 0.38, y: length - 2), control: CGPoint(x: 0, y: length + 14))
                p.addCurve(to: CGPoint(x: width * 0.42, y: 2), control1: CGPoint(x: width * 0.43, y: length * 0.72), control2: CGPoint(x: width * 0.64, y: length * 0.3))
                p.addQuadCurve(to: CGPoint(x: -width * 0.42, y: 2), control: CGPoint(x: 0, y: -15))
                p.closeSubpath()
            }
            gradient(shape, colors: [paint.dark, paint.mid, paint.light, paint.mid, paint.dark],
                     from: CGPoint(x: -width / 2, y: 0), to: CGPoint(x: width / 2, y: 0))
            stroke(line(CGPoint(x: -width * 0.18, y: 12), CGPoint(x: -width * 0.18, y: length - 15)), rgb(0xffffff, alpha: 0.85), width: width * 0.12)
        }
    }

    func limb(_ shoulder: CGPoint, _ elbow: CGPoint, _ wrist: CGPoint,
              rootDepth: CGFloat, elbowDepth: CGFloat, endDepth: CGFloat, width: CGFloat, paint: Enamel) {
        enqueue(depth: rootDepth + 0.3) { joint(shoulder, radius: width * 0.51) }
        layers.addSegment(from: shoulder, to: elbow, startDepth: rootDepth, endDepth: elbowDepth, width: width) { _ in
            segment(shoulder, elbow, width: width, paint: paint)
        }
        enqueue(depth: elbowDepth + 0.3) { joint(elbow, radius: width * 0.46) }
        layers.addSegment(from: elbow, to: wrist, startDepth: elbowDepth, endDepth: endDepth, width: width) { _ in
            segment(elbow, wrist, width: width * 0.88, paint: paint)
        }
        enqueue(depth: endDepth + 0.3) { joint(wrist, radius: width * 0.35) }
    }

    func foot(_ x: CGFloat, _ y: CGFloat, paint: Enamel, angle: CGFloat = 0, scale: CGFloat = 1) {
        at(x, y, angle: angle, scale: scale) {
            let sole = path { p in
                p.move(to: CGPoint(x: -38, y: -12))
                p.addCurve(to: CGPoint(x: 43, y: 12), control1: CGPoint(x: -17, y: -34), control2: CGPoint(x: 48, y: -21))
                p.addCurve(to: CGPoint(x: -43, y: 8), control1: CGPoint(x: 48, y: 35), control2: CGPoint(x: -49, y: 34))
                p.closeSubpath()
            }
            gradient(sole, colors: [rgb(0x6b6270), rgb(0x242735)])
            cg.translateBy(x: 0, y: -8)
            enamel(sole, paint)
            stroke(path { p in
                p.move(to: CGPoint(x: -31, y: -9))
                p.addQuadCurve(to: CGPoint(x: 30, y: 2), control: CGPoint(x: 0, y: -16))
            }, rgb(0xffffff, alpha: 0.85), width: 5)
        }
    }

    func hand(_ x: CGFloat, _ y: CGFloat, paint: Enamel, angle: CGFloat = 0, scale: CGFloat = 1) {
        at(x, y, angle: angle, scale: scale) {
            let shape = path { p in
                p.move(to: CGPoint(x: -19, y: -17))
                p.addQuadCurve(to: CGPoint(x: -37, y: 12), control: CGPoint(x: -22, y: 5))
                p.addLine(to: CGPoint(x: -53, y: 24))
                p.addCurve(to: CGPoint(x: -41, y: 40), control1: CGPoint(x: -69, y: 37), control2: CGPoint(x: -52, y: 52))
                p.addLine(to: CGPoint(x: -23, y: 27))
                p.addLine(to: CGPoint(x: -34, y: 50))
                p.addCurve(to: CGPoint(x: -16, y: 58), control1: CGPoint(x: -43, y: 67), control2: CGPoint(x: -23, y: 73))
                p.addLine(to: CGPoint(x: -5, y: 35))
                p.addLine(to: CGPoint(x: -6, y: 62))
                p.addCurve(to: CGPoint(x: 13, y: 63), control1: CGPoint(x: -6, y: 80), control2: CGPoint(x: 15, y: 80))
                p.addLine(to: CGPoint(x: 16, y: 36))
                p.addLine(to: CGPoint(x: 26, y: 56))
                p.addCurve(to: CGPoint(x: 44, y: 48), control1: CGPoint(x: 34, y: 72), control2: CGPoint(x: 53, y: 62))
                p.addLine(to: CGPoint(x: 30, y: 16))
                p.addQuadCurve(to: CGPoint(x: 22, y: -17), control: CGPoint(x: 21, y: 4))
                p.addQuadCurve(to: CGPoint(x: -19, y: -17), control: CGPoint(x: 2, y: -27))
                p.closeSubpath()
            }
            enamel(shape, paint, border: 5.5)
            stroke(path { p in
                p.move(to: CGPoint(x: -11, y: -8))
                p.addQuadCurve(to: CGPoint(x: -37, y: 30), control: CGPoint(x: -14, y: 14))
            }, rgb(0xffffff, alpha: 0.9), width: 6)
            stroke(line(CGPoint(x: 1, y: 28), CGPoint(x: -19, y: 53)), rgb(0xffffff, alpha: 0.5), width: 4)
        }
    }

    func torso(_ x: CGFloat, _ y: CGFloat, angle: CGFloat, paint: Enamel, scale: CGFloat = 1) {
        at(x, y, angle: angle, scale: scale) {
            enamel(rounded(-66, -48, 132, 112, 53), paint)
            enamel(rounded(-49, -42, 98, 89, 40), paint, border: 3)
            gradient(rounded(-30, -58, 60, 28, 12), colors: [rgb(0xaaa1a9), rgb(0x3a3440)])
        }
    }

    func head(_ x: CGFloat, _ y: CGFloat, angle: CGFloat, paint: Enamel, robot: PortDanceChoreography.Robot,
              time: TimeInterval, phase: Double, scale: CGFloat = 1, turn: CGFloat = 0, smiling: Bool = false) {
        at(x, y, angle: angle, scale: scale) {
            cg.scaleBy(x: 1 - abs(turn) * 0.08, y: 1)
            // Antenna and earpieces sit behind the face shell.
            if !smiling {
                segment(CGPoint(x: -24, y: -96), CGPoint(x: -29, y: -153), width: 25, paint: paint)
                enamel(ellipse(-59, -196, 60, 60), paint, border: 6)
                fill(ellipse(-44, -186, 18, 14), white)
            }
            gradient(ellipse(-165, -20, 55, 76), colors: [paint.light, paint.dark, rgb(0x42353c)])
            gradient(ellipse(110, -20, 55, 76), colors: [paint.light, paint.dark, rgb(0x42353c)])
            let shell = rounded(-142, -109, 284, 222, 83)
            saved {
                cg.setShadow(offset: CGSize(width: 2, height: 7), blur: 8, color: rgb(0x281f2e, alpha: 0.24))
                fill(shell, paint.dark)
            }
            enamel(shell, paint, border: 6)
            // The broad white reflection follows the shell's top curve.
            let shine = path { p in
                p.move(to: CGPoint(x: -127, y: -47))
                p.addCurve(to: CGPoint(x: 103, y: -73), control1: CGPoint(x: -111, y: -126), control2: CGPoint(x: 77, y: -122))
                p.addCurve(to: CGPoint(x: -92, y: -58), control1: CGPoint(x: 60, y: -104), control2: CGPoint(x: -68, y: -92))
                p.addQuadCurve(to: CGPoint(x: -127, y: -47), control: CGPoint(x: -119, y: -39))
                p.closeSubpath()
            }
            fill(shine, rgb(0xffffff, alpha: 0.92))
            stroke(path { p in
                p.move(to: CGPoint(x: -125, y: -20))
                p.addQuadCurve(to: CGPoint(x: -104, y: 72), control: CGPoint(x: -138, y: 53))
            }, rgb(0xffffff, alpha: 0.48), width: 4)
            let rim = rounded(-118, -42, 236, 137, 43)
            gradient(rim, colors: [paint.light, paint.mid], border: 0)
            let screen = rounded(-108, -34, 216, 119, 35)
            gradient(screen, colors: [rgb(0x152f34), rgb(0x07191e), rgb(0x04090c)], border: 5)
            stroke(path { p in
                p.move(to: CGPoint(x: -94, y: 7))
                p.addQuadCurve(to: CGPoint(x: -70, y: -22), control: CGPoint(x: -92, y: -21))
                p.addLine(to: CGPoint(x: 64, y: -22))
            }, rgb(0xffffff, alpha: 0.09), width: 3)
            let cycle = (time + phase).truncatingRemainder(dividingBy: 6.4)
            let blink = max(0.09, min(1, abs(cycle - 4.9) / 0.12))
            let target = PortDanceStage.project(frame.move.target)
            let dx = target.x - x
            let dy = target.y - y
            let distance = max(1, hypot(dx, dy))
            let radians = angle * .pi / 180
            let gaze = CGPoint(x: (dx * cos(radians) + dy * sin(radians)) / distance * 7,
                               y: (-dx * sin(radians) + dy * cos(radians)) / distance * 5)
            let reacting = frame.reaction(robot) > 0.1
            for eyeX: CGFloat in [-44, 44] {
                saved {
                    cg.translateBy(x: gaze.x, y: gaze.y)
                    cg.setShadow(offset: .zero, blur: 17, color: paint.glow.copy(alpha: 0.8))
                    if reacting && frame.outcome(robot) == .won {
                        stroke(path { p in
                            p.move(to: CGPoint(x: eyeX - 18, y: 34))
                            p.addQuadCurve(to: CGPoint(x: eyeX + 18, y: 34), control: CGPoint(x: eyeX, y: 4))
                        }, paint.glow, width: 9)
                    } else if reacting && frame.outcome(robot) == .lost {
                        // One overcommitted robot squints; the hesitant one gives a wide-eyed double take.
                        if frame.rank(robot) == 1 {
                            fill(ellipse(eyeX - 18, 15, 36, 29), paint.glow)
                            stroke(line(CGPoint(x: eyeX - 21, y: 13), CGPoint(x: eyeX + 20, y: 25)),
                                   rgb(0x10272c), width: 10)
                        } else {
                            stroke(ellipse(eyeX - 17, 7, 34, 43), paint.glow, width: 8)
                        }
                    } else if smiling {
                        let direction: CGFloat = eyeX < 0 ? 1 : -1
                        let chevron = path { p in
                            p.move(to: CGPoint(x: eyeX - 14 * direction, y: 9))
                            p.addLine(to: CGPoint(x: eyeX + 11 * direction, y: 27))
                            p.addLine(to: CGPoint(x: eyeX - 14 * direction, y: 45))
                        }
                        stroke(chevron, paint.glow, width: 10)
                    } else {
                        fill(ellipse(eyeX - 20, 26 - 30 * blink, 40, 60 * blink), paint.glow)
                    }
                }
            }
            stroke(line(CGPoint(x: -13, y: 101), CGPoint(x: 13, y: 101)), ink, width: 5)
        }
    }

    func robot(_ robot: PortDanceChoreography.Robot, time: TimeInterval) {
        let body = bodies[robot]!
        let rig = PortDanceStage.rig(robot)
        let paint: Enamel = switch robot { case .blue: .blue; case .gold: .gold; case .rose: .rose }
        let chest = body.point(rig.center)
        enqueue(depth: bodyDepth(robot)) {
            torso(chest.x, chest.y, angle: body.angle, paint: paint, scale: body.scale * (robot == .rose ? 0.82 : 1))
        }
        for side in PortDanceChoreography.Limb.allCases {
            let contact = frame.position(.init(robot: robot, limb: side))
            let root = body.point(rig.root(side))
            let handScale: CGFloat = robot == .rose ? 0.8 : 0.9
            let angle: CGFloat = side.isHand ? (side.isLeft ? -10 : 10) : (side.isLeft ? 8 : -8)
            let pose = PortDanceStage.articulation(robot, side, body: body, frame: frame)
            let end = pose.end.screen
            let elbow = pose.joint.screen
            let lift = frame.elevation(robot, side)
            let depth = pose.end.y
            let rootDepth = pose.root.y
            let elbowDepth = pose.joint.y
            limb(root, elbow, end, rootDepth: rootDepth, elbowDepth: elbowDepth, endDepth: depth,
                 width: rig.width, paint: paint)
            contactShadow(contact, elevation: lift, width: 90 * handScale, height: 26 * handScale)
            enqueue(depth: depth + 0.8) {
                if side.isHand {
                    let offset = PortDanceStage.palmContact(.zero, angle: angle, scale: handScale)
                    hand(contact.x - offset.x, contact.y - offset.y, paint: paint, angle: angle, scale: handScale)
                } else {
                    let radians = angle * .pi / 180
                    foot(contact.x + sin(radians) * 8 * handScale, contact.y - cos(radians) * 8 * handScale,
                         paint: paint, angle: angle, scale: handScale)
                }
            }
        }
        let face = body.point(rig.face)
        if robot == .rose {
            enqueue(depth: bodyDepth(robot) + 0.5) {
                at(face.x + 36, face.y - 90, angle: body.angle - 15, scale: body.scale * 0.8) {
                    enamel(ellipse(-33, -28, 69, 72), paint)
                }
            }
        }
        enqueue(depth: bodyDepth(robot) + 1) {
            head(face.x, face.y, angle: rig.headAngle + body.angle * 0.7, paint: paint, robot: robot,
                 time: time, phase: robot == .blue ? 0 : (robot == .gold ? 2.7 : 1),
                 scale: rig.headScale * body.scale, turn: body.turn, smiling: robot == .rose)
        }
    }
}
