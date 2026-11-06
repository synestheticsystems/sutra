import AppKit
import ImageIO
import SwiftUI
import UniformTypeIdentifiers

/// Standalone artwork preview: no Rust core, project scanning, or Sutra preferences.
@main
struct PortDancePreview {
    @MainActor
    static func main() throws {
        let arguments = Array(CommandLine.arguments.dropFirst())
        if let mode = arguments.first, mode == "--snapshot" || mode == "--gif" {
            guard arguments.count >= 2 else { throw PreviewError.usage }
            let output = URL(fileURLWithPath: arguments[1])
            let transparent = arguments.contains("--transparent")
            let size = mode == "--gif" ? 640 : 2048
            let duration: Double
            if let index = arguments.firstIndex(of: "--duration") {
                guard index + 1 < arguments.count, let value = Double(arguments[index + 1]),
                      value.isFinite, value >= 0.1, value <= 120 else { throw PreviewError.usage }
                duration = value
            } else { duration = 24 }
            let frameRate = 20.0
            let frameCount = mode == "--gif" ? Int((duration * frameRate).rounded()) : 1
            let seed: UInt64
            if let index = arguments.firstIndex(of: "--seed") {
                guard index + 1 < arguments.count, let value = UInt64(arguments[index + 1]) else { throw PreviewError.usage }
                seed = value
            } else { seed = PortDanceChoreography.exportSeed }
            let time: Double
            if let index = arguments.firstIndex(of: "--time") {
                guard index + 1 < arguments.count, let value = Double(arguments[index + 1]), value.isFinite, value >= 0 else { throw PreviewError.usage }
                time = value
            } else { time = 0 }
            let type = mode == "--gif" ? UTType.gif.identifier : UTType.png.identifier
            guard let destination = CGImageDestinationCreateWithURL(output as CFURL, type as CFString, frameCount, nil) else {
                throw PreviewError.cannotCreateOutput
            }
            if mode == "--gif" {
                CGImageDestinationSetProperties(destination, [kCGImagePropertyGIFDictionary: [kCGImagePropertyGIFLoopCount: 0]] as CFDictionary)
            }
            for frame in 0..<frameCount {
                guard let context = CGContext(data: nil, width: size, height: size, bitsPerComponent: 8,
                                              bytesPerRow: 0, space: CGColorSpace(name: CGColorSpace.sRGB)!,
                                              bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { throw PreviewError.cannotRender }
                context.translateBy(x: 0, y: CGFloat(size))
                context.scaleBy(x: 1, y: -1)
                if !transparent {
                    context.setFillColor(CGColor(srgbRed: 0.99, green: 0.96, blue: 0.93, alpha: 1))
                    context.fill(CGRect(x: 0, y: 0, width: size, height: size))
                }
                PortDanceDrawing.draw(in: context, size: CGSize(width: size, height: size), time: time + Double(frame) / frameRate, seed: seed, showsCue: mode == "--gif" || arguments.contains("--cue"))
                guard let image = context.makeImage() else { throw PreviewError.cannotRender }
                let properties = mode == "--gif" ? [kCGImagePropertyGIFDictionary: [kCGImagePropertyGIFDelayTime: 1 / frameRate]] as CFDictionary : nil
                CGImageDestinationAddImage(destination, image, properties)
            }
            guard CGImageDestinationFinalize(destination) else { throw PreviewError.cannotCreateOutput }
            print(output.path)
        } else if arguments.isEmpty {
            PreviewApp.main()
        } else {
            throw PreviewError.usage
        }
    }
}

private enum PreviewError: Error {
    case usage, cannotCreateOutput, cannotRender
}

private struct PreviewApp: App {
    @State private var seen = NSApp.occlusionState.contains(.visible)
    var body: some Scene {
        Window("Robo Sutra · Welcome", id: "artwork") {
            IntroView { NSApp.terminate(nil) }
                .environment(\.motion, seen ? .live : .steady)
                .onReceive(NotificationCenter.default.publisher(for: NSApplication.didChangeOcclusionStateNotification)) { _ in
                    seen = NSApp.occlusionState.contains(.visible)
                }
        }
        .defaultSize(width: 520, height: 640)
    }
}
