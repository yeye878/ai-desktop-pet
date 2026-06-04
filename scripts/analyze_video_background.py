#!/usr/bin/env python3
"""分析视频背景颜色"""

from pathlib import Path
import sys

# 添加opencv和numpy支持
try:
    import cv2
    import numpy as np
    from PIL import Image
except ImportError as e:
    print(f"缺少依赖: {e}")
    print("请安装: pip install opencv-python numpy pillow")
    sys.exit(1)


def analyze_video_background(video_path: Path):
    """分析视频的背景颜色"""
    
    if not video_path.exists():
        print(f"视频文件不存在: {video_path}")
        return
    
    # 打开视频
    cap = cv2.VideoCapture(str(video_path))
    if not cap.isOpened():
        print(f"无法打开视频: {video_path}")
        return
    
    # 获取视频信息
    width = int(cap.get(cv2.CAP_PROP_FRAME_WIDTH))
    height = int(cap.get(cv2.CAP_PROP_FRAME_HEIGHT))
    fps = cap.get(cv2.CAP_PROP_FPS)
    total_frames = int(cap.get(cv2.CAP_PROP_FRAME_COUNT))
    
    print(f"视频信息:")
    print(f"  分辨率: {width}x{height}")
    print(f"  帧率: {fps}")
    print(f"  总帧数: {total_frames}")
    print()
    
    # 读取第一帧
    ret, frame = cap.read()
    if not ret:
        print("无法读取视频帧")
        cap.release()
        return
    
    cap.release()
    
    # 转换为RGB
    frame_rgb = cv2.cvtColor(frame, cv2.COLOR_BGR2RGB)
    
    # 分析四个角和边缘的颜色
    print("分析四个角的颜色:")
    corners = [
        ("左上角", (0, 0, 10, 10)),
        ("右上角", (width-10, 0, width, 10)),
        ("左下角", (0, height-10, 10, height)),
        ("右下角", (width-10, height-10, width, height)),
    ]
    
    for name, (x1, y1, x2, y2) in corners:
        corner = frame_rgb[y1:y2, x1:x2]
        avg_color = corner.mean(axis=(0, 1))
        print(f"  {name}: RGB({avg_color[0]:.1f}, {avg_color[1]:.1f}, {avg_color[2]:.1f})")
    
    # 分析边缘区域（上下左右各10像素）
    print("\n分析边缘区域的颜色:")
    edges = [
        ("上边缘", (0, 0, width, 10)),
        ("下边缘", (0, height-10, width, height)),
        ("左边缘", (0, 0, 10, height)),
        ("右边缘", (width-10, 0, width, height)),
    ]
    
    for name, (x1, y1, x2, y2) in edges:
        edge = frame_rgb[y1:y2, x1:x2]
        avg_color = edge.mean(axis=(0, 1))
        min_color = edge.min(axis=(0, 1))
        max_color = edge.max(axis=(0, 1))
        print(f"  {name}:")
        print(f"    平均: RGB({avg_color[0]:.1f}, {avg_color[1]:.1f}, {avg_color[2]:.1f})")
        print(f"    范围: RGB({min_color[0]}-{max_color[0]}, {min_color[1]}-{max_color[1]}, {min_color[2]}-{max_color[2]})")
    
    # 分析整体颜色分布
    print("\n分析整体颜色分布:")
    # 采样中心区域
    center = frame_rgb[height//4:3*height//4, width//4:3*width//4]
    avg_color = center.mean(axis=(0, 1))
    print(f"  中心区域平均: RGB({avg_color[0]:.1f}, {avg_color[1]:.1f}, {avg_color[2]:.1f})")
    
    # 保存第一帧图像供查看
    output_path = Path(__file__).parent.parent / "src" / "assets" / "pets" / "daimao-batiao" / "video_frame_0.jpg"
    output_path.parent.mkdir(parents=True, exist_ok=True)
    cv2.imwrite(str(output_path), frame)
    print(f"\n已保存第一帧到: {output_path}")
    
    # 分析透明度建议
    print("\n透明度处理建议:")
    corner_avg = np.array([
        frame_rgb[0:10, 0:10].mean(axis=(0, 1)),
        frame_rgb[0:10, width-10:width].mean(axis=(0, 1)),
        frame_rgb[height-10:height, 0:10].mean(axis=(0, 1)),
        frame_rgb[height-10:height, width-10:width].mean(axis=(0, 1)),
    ]).mean(axis=0)
    
    print(f"  角落平均颜色: RGB({corner_avg[0]:.1f}, {corner_avg[1]:.1f}, {corner_avg[2]:.1f})")
    
    # 判断背景颜色
    if corner_avg[0] > 240 and corner_avg[1] > 240 and corner_avg[2] > 240:
        print("  背景判断: 白色或接近白色")
        print("  建议: 使用颜色距离算法，只透明化接近白色的像素")
    elif corner_avg[1] > 200 and corner_avg[1] > corner_avg[0] and corner_avg[1] > corner_avg[2]:
        print("  背景判断: 绿色背景（绿幕）")
        print("  建议: 使用色度键控，透明化绿色像素")
    elif corner_avg[2] > 200 and corner_avg[2] > corner_avg[0] and corner_avg[2] > corner_avg[1]:
        print("  背景判断: 蓝色背景（蓝幕）")
        print("  建议: 使用色度键控，透明化蓝色像素")
    else:
        print("  背景判断: 其他颜色")
        print("  建议: 使用颜色距离算法，计算与背景颜色的差异")


if __name__ == "__main__":
    video_path = Path("c:/Users/15188/Desktop/素材/4e513af41b45d29692ff6abeb4c1e0ae.mp4")
    analyze_video_background(video_path)
