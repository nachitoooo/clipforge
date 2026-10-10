// Test oracle: how AVFoundation (the engine under QuickTime and Safari's
// <video>) sees the first video track of an .mp4 — edit lists included.
// Prints one line: `duration=<s> frames=<n> first=<s> last=<s>`, where the
// frames are really decoded, in presentation order.
//
// usage: avf_video <in.mp4>
import AVFoundation
import Foundation

let args = CommandLine.arguments
guard args.count == 2 else {
    FileHandle.standardError.write("usage: avf_video <in.mp4>\n".data(using: .utf8)!)
    exit(2)
}
let asset = AVURLAsset(url: URL(fileURLWithPath: args[1]))
guard let track = asset.tracks(withMediaType: .video).first else {
    FileHandle.standardError.write("no video track\n".data(using: .utf8)!)
    exit(3)
}
let reader = try AVAssetReader(asset: asset)
let output = AVAssetReaderTrackOutput(
    track: track,
    outputSettings: [kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA])
reader.add(output)
guard reader.startReading() else {
    FileHandle.standardError.write("cannot read: \(String(describing: reader.error))\n".data(using: .utf8)!)
    exit(4)
}
var frames = 0
var first = Double.nan
var last = Double.nan
while let sample = output.copyNextSampleBuffer() {
    let pts = CMSampleBufferGetPresentationTimeStamp(sample).seconds
    if frames == 0 { first = pts }
    last = pts
    frames += 1
}
if reader.status == .failed {
    FileHandle.standardError.write("read failed: \(String(describing: reader.error))\n".data(using: .utf8)!)
    exit(5)
}
print(String(format: "duration=%.4f frames=%d first=%.4f last=%.4f", asset.duration.seconds, frames, first, last))
