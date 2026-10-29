import AppKit
import Foundation
let output = URL(fileURLWithPath: CommandLine.arguments[1], isDirectory: true)
try FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)
for (size,name) in [(16,"icon_16x16"),(32,"icon_16x16@2x"),(32,"icon_32x32"),(64,"icon_32x32@2x"),(128,"icon_128x128"),(256,"icon_128x128@2x"),(256,"icon_256x256"),(512,"icon_256x256@2x"),(512,"icon_512x512"),(1024,"icon_512x512@2x")] {
    let bitmap=NSBitmapImageRep(bitmapDataPlanes:nil,pixelsWide:size,pixelsHigh:size,bitsPerSample:8,samplesPerPixel:4,hasAlpha:true,isPlanar:false,colorSpaceName:.deviceRGB,bytesPerRow:0,bitsPerPixel:0)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current=NSGraphicsContext(bitmapImageRep:bitmap)
    let scale=CGFloat(size)/1024
    let transform=NSAffineTransform();transform.scale(by:scale);transform.concat()
    let background=NSBezierPath(roundedRect:NSRect(x:24,y:24,width:976,height:976),xRadius:220,yRadius:220)
    NSGradient(starting:NSColor(calibratedRed:0.94,green:0.96,blue:0.90,alpha:1),ending:NSColor.white)!.draw(in:background,angle:70)
    NSColor(calibratedRed:0.20,green:0.40,blue:0.32,alpha:1).setFill()
    for index in 0..<6 {
        NSGraphicsContext.saveGraphicsState()
        let t=NSAffineTransform();t.translateX(by:512,yBy:550);t.rotate(byDegrees:CGFloat(index)*60);t.concat()
        NSBezierPath(roundedRect:NSRect(x:-30,y:-245,width:60,height:490),xRadius:30,yRadius:30).fill()
        NSGraphicsContext.restoreGraphicsState()
    }
    let text="P" as NSString
    text.draw(at:NSPoint(x:463,y:155),withAttributes:[.font:NSFont.systemFont(ofSize:130,weight:.bold),.foregroundColor:NSColor(calibratedRed:0.20,green:0.40,blue:0.32,alpha:1)])
    NSGraphicsContext.restoreGraphicsState()
    try bitmap.representation(using:.png,properties:[:])!.write(to:output.appendingPathComponent(name+".png"))
}
