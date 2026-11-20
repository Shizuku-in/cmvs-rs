import Foundation
import ImageIO
import CoreGraphics

let url = URL(fileURLWithPath: CommandLine.arguments[1])
guard let source = CGImageSourceCreateWithURL(url as CFURL,nil),
      let image = CGImageSourceCreateImageAtIndex(source,0,nil),
      image.width == 64, image.height == 48 else { fatalError("Missing 64×48 logical frame") }
var pixels = [UInt8](repeating:0,count:64*48*4)
let visible = pixels.withUnsafeMutableBytes { bytes in
    let context = CGContext(data:bytes.baseAddress,width:64,height:48,bitsPerComponent:8,
                            bytesPerRow:64*4,space:CGColorSpaceCreateDeviceRGB(),
                            bitmapInfo:CGImageAlphaInfo.premultipliedLast.rawValue)!
    context.draw(image,in:CGRect(x:0,y:0,width:64,height:48))
    return stride(from:0,to:bytes.count,by:4).contains { bytes[$0] != 0 || bytes[$0+1] != 0 || bytes[$0+2] != 0 }
}
precondition(visible, "Completed logical frame was black")
