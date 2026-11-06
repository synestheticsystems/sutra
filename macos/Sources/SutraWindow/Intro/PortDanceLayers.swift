import CoreGraphics

/// A shared back-to-front list for every robot part. A long limb is clipped into depth
/// slices so its far end can pass behind a body while its near end remains in front.
final class PortDanceLayers {
    private struct Layer {
        let depth: CGFloat
        let order: Int
        let draw: (CGContext) -> Void
    }

    private var layers: [Layer] = []
    private struct Segment {
        let start: CGPoint
        let end: CGPoint
        let startDepth: CGFloat
        let endDepth: CGFloat
        let width: CGFloat
        let draw: (CGContext) -> Void
        var length: CGFloat { max(0.001, hypot(end.x - start.x, end.y - start.y)) }
        func depth(_ t: CGFloat) -> CGFloat { startDepth + (endDepth - startDepth) * t }
    }
    private struct Crossing {
        let center: CGFloat
        let radius: CGFloat
        let depth: CGFloat
    }
    private var segments: [Segment] = []

    func add(depth: CGFloat, draw: @escaping (CGContext) -> Void) {
        layers.append(Layer(depth: depth, order: layers.count, draw: draw))
    }

    func addSegment(from start: CGPoint, to end: CGPoint, startDepth: CGFloat, endDepth: CGFloat,
                    width: CGFloat, draw: @escaping (CGContext) -> Void) {
        segments.append(Segment(start: start, end: end, startDepth: startDepth, endDepth: endDepth,
                                width: width, draw: draw))
    }

    /// One crossing must have one consistent front limb across its entire width. Using only
    /// each slice's depth can otherwise expose triangular slivers of the rear limb.
    private func crossings(_ a: Segment, among others: [Segment]) -> [Crossing] {
        let ax = a.end.x - a.start.x
        let ay = a.end.y - a.start.y
        return others.compactMap { b in
            let bx = b.end.x - b.start.x
            let by = b.end.y - b.start.y
            let determinant = ax * by - ay * bx
            let sine = abs(determinant) / (a.length * b.length)
            if a.start == b.start && a.end == b.end { return nil }
            // Folded upper/lower sections overlap beside their shared joint. Keep that
            // overlap at the joint's depth too, so one section cannot nick the other.
            let shared: CGFloat? = a.start == b.end ? 0 : (a.end == b.start ? 1 : nil)
            if let shared {
                let radius = (a.width + b.width) * 0.6 / max(0.1, sine) / a.length
                return Crossing(center: shared, radius: radius, depth: a.depth(shared))
            }
            guard abs(determinant) > 0.001 else { return nil }
            let dx = b.start.x - a.start.x
            let dy = b.start.y - a.start.y
            let t = (dx * by - dy * bx) / determinant
            let u = (dx * ay - dy * ax) / determinant
            // Adjacent segments meet at a joint rather than crossing one another.
            guard t > 0.01, t < 0.99, u > 0.01, u < 0.99 else { return nil }
            let radius = (a.width + b.width) * 0.6 / max(0.1, sine) / a.length
            return Crossing(center: t, radius: radius, depth: a.depth(t))
        }
    }

    private func slice(_ segment: Segment, crossings: [Crossing]) {
        let (start, end, width) = (segment.start, segment.end, segment.width)
        let length = max(0.001, hypot(end.x - start.x, end.y - start.y))
        let dx = (end.x - start.x) / length
        let dy = (end.y - start.y) / length
        let count = max(1, Int(ceil(length / 32)))
        for index in 0..<count {
            let lo = CGFloat(index) / CGFloat(count)
            let hi = CGFloat(index + 1) / CGFloat(count)
            let near = index == 0 ? -width : lo * length - 0.3
            let far = index == count - 1 ? length + width : hi * length + 0.3
            func point(_ along: CGFloat, _ across: CGFloat) -> CGPoint {
                CGPoint(x: start.x + dx * along - dy * across,
                        y: start.y + dy * along + dx * across)
            }
            let clip = CGMutablePath()
            clip.move(to: point(near, -width))
            clip.addLine(to: point(far, -width))
            clip.addLine(to: point(far, width))
            clip.addLine(to: point(near, width))
            clip.closeSubpath()
            let middle = (lo + hi) / 2
            let crossing = crossings.filter { hi >= $0.center - $0.radius && lo <= $0.center + $0.radius }
                .min { abs($0.center - middle) < abs($1.center - middle) }
            add(depth: crossing?.depth ?? segment.depth(middle)) { cg in
                cg.setShouldAntialias(false)
                cg.addPath(clip)
                cg.clip()
                cg.setShouldAntialias(true)
                segment.draw(cg)
            }
        }
    }

    func render(in cg: CGContext) {
        let pending = segments
        segments.removeAll()
        for segment in pending { slice(segment, crossings: crossings(segment, among: pending)) }
        let sorted = layers.sorted { a, b in
            a.depth == b.depth ? a.order < b.order : a.depth < b.depth
        }
        // Release queued closures after drawing, including closures that retain the painter.
        layers.removeAll()
        for layer in sorted {
            cg.saveGState()
            layer.draw(cg)
            cg.restoreGState()
        }
    }
}
