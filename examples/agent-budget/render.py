#!/usr/bin/env python3
"""Render the real cast plus clearly labeled future-concept title cards.
Requires Pillow, pyte==0.8.2 and imageio-ffmpeg==0.6.0. No synthesized CLI output.
"""
import argparse
import json
from pathlib import Path
import subprocess
import textwrap
import imageio_ffmpeg
import pyte
from PIL import Image, ImageDraw, ImageFont

ROOT=Path(__file__).resolve().parents[2]
W,H,FPS=1280,800,12
BG='#091118'; PANEL='#101e29'; FG='#e7f1f5'; MUTED='#9cb2c1'; GREEN='#70e6b0'; AMBER='#ffc56c'

def font(size,mono=False):
    paths = (['/System/Library/Fonts/Menlo.ttc','/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf'] if mono else
             ['/System/Library/Fonts/Supplemental/Arial.ttf','/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf'])
    for path in paths:
        if Path(path).exists(): return ImageFont.truetype(path,size)
    raise RuntimeError('Install DejaVu Sans / DejaVu Sans Mono')

def shell(tag,title,subtitle,accent=GREEN):
    im=Image.new('RGB',(W,H),BG); d=ImageDraw.Draw(im)
    d.text((48,30),'zwallet',font=font(25,True),fill=FG)
    d.rounded_rectangle((820,25,1232,63),radius=8,fill=PANEL)
    d.text((838,33),tag,font=font(17,True),fill=accent)
    title_size=38
    while d.textbbox((0,0),title,font=font(title_size))[2]>1184 and title_size>26:
        title_size-=1
    d.text((48,90),title,font=font(title_size),fill=FG)
    d.text((48,143),subtitle,font=font(19),fill=MUTED)
    d.line((48,183,1232,183),fill='#284050',width=1)
    d.text((48,757),'2lab-ai/zwallet  |  MIT  |  Local keys. Your agent. Your RPC.',font=font(18,True),fill=MUTED)
    return im,d

def card(title,subtitle,lines,future=False):
    im,d=shell('CONCEPT / WIP - NOT AVAILABLE' if future else 'OPEN SOURCE / AVAILABLE TODAY',title,subtitle,AMBER if future else GREEN)
    for n,(label,body) in enumerate(lines):
        y=221+n*147
        d.text((60,y),label.upper(),font=font(16,True),fill=AMBER if future else GREEN)
        for j,line in enumerate(textwrap.wrap(body,66)):
            d.text((60,y+31+j*37),line,font=font(28),fill=FG)
    return im

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--assets',type=Path,default=ROOT/'docs/assets')
    a=p.parse_args(); assets=a.assets
    entries=[json.loads(l) for l in (assets/'agent-budget.cast').read_text().splitlines()]
    events=entries[1:]; proof=json.loads((assets/'agent-budget-evidence.json').read_text())
    # Trim only pre-recording setup; preserve observed delays within the recorded workflow.
    offset=events[0][0]
    duration=proof['duration']-offset
    intro=card('Fund the task. Let your agent execute.',
               'Local keys. Your rules. Verifiable receipts.',[
                   ('The problem','The agent writes code. You still move the money.'),
                   ('The workflow','Fund a task. Set rules. Unlock. Get receipts.'),
                   ('The proof','Real CLI. Local Anvil. Valueless ETH. Scripted workflow, not an LLM session.')])
    future1=card('Next: an investment agent with boundaries.',
                 'Future product scenario. No trade is executed in this segment.',[
                     ('You define the mandate','50 USDT. An approved strategy, assets and venue. Explicit exposure limits.'),
                     ('The agent plans; the wallet enforces','Validate the quote, slippage and remaining budget before execution.'),
                     ('You get evidence','Positions, fees and P&L. Returns can be negative; no profit promise.')],True)
    future2=card('Next: ask for a purchase, get an order.',
                 'Future product scenario. Merchant and payment adapters are not implemented.',[
                     ('You define the purchase','Buy a hardware tool from an approved seller, within a 60 USDT-equivalent budget.'),
                     ('The agent handles the workflow','Compare all-in quotes. Include delivery and fees. Pay once under policy.'),
                     ('You get the result','Order, payment and refund status. Shipping data stays outside the signer.')],True)
    outro=card('Build with it today. Shape what comes next.',
               'Try the reproducible local demo. Tell us what your agent needs to pay for.',[
                   ('Today','EVM commands. Signed rules. Per-transaction limits. Timed sessions. Receipts.'),
                   ('WIP','Task budgets. Investment, payment and shopping adapters. Native spend reports.'),
                   ('Start here','github.com/2lab-ai/zwallet')])
    ff=imageio_ffmpeg.get_ffmpeg_exe()
    proc=subprocess.Popen([ff,'-y','-loglevel','error','-f','rawvideo','-vcodec','rawvideo',
                           '-pix_fmt','rgb24','-s',f'{W}x{H}','-r',str(FPS),'-i','-',
                           '-an','-c:v','libx264','-preset','medium','-crf','22',
                           '-pix_fmt','yuv420p','-movflags','+faststart',str(assets/'agent-budget.mp4')],stdin=subprocess.PIPE)
    thumbs=[]
    def frames(im,seconds):
        for _ in range(round(seconds*FPS)): proc.stdin.write(im.tobytes())
    frames(intro,6); thumbs.append(intro)
    screen=pyte.Screen(104,24); stream=pyte.Stream(screen); idx=0
    seen=set()
    capture_times={s['label']: (proof['scenes'][n+1]['at'] if n+1<len(proof['scenes']) else proof['duration'])-.2
                   for n,s in enumerate(proof['scenes'])}
    for frame in range(int((duration+1)*FPS)):
        now=offset+frame/FPS
        while idx<len(events) and events[idx][0]<=now:
            stream.feed(events[idx][2]); idx+=1
        scenes=[s for s in proof['scenes'] if s['at']<=now]
        label=scenes[-1]['label'] if scenes else 'Real CLI workflow'
        im,d=shell('REAL CLI / LOCAL ANVIL',label,
                   'Recorded command output, selected JSON fields. Valueless ETH; no public transaction.')
        d.rounded_rectangle((43,204,1237,737),radius=12,fill=PANEL)
        for line,text in enumerate(screen.display):
            d.text((61,217+line*21),text,font=font(18,True),fill=GREEN if text.startswith('$') else FG)
        proc.stdin.write(im.tobytes())
        # Capture a legible completed screen for each chapter, not its first empty frame.
        if scenes and now>=capture_times[label] and label not in seen:
            thumbs.append(im.copy()); seen.add(label)
    frames(future1,10); thumbs.append(future1)
    frames(future2,10); thumbs.append(future2)
    frames(outro,7); thumbs.append(outro)
    proc.stdin.close()
    if proc.wait()!=0: raise RuntimeError('MP4 encoding failed')
    # Compact technical preview; the public README uses the separate agent-story explainer.
    subprocess.run([ff,'-y','-loglevel','error','-i',str(assets/'agent-budget.mp4'),
                    '-filter_complex','fps=4,scale=960:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=96[p];[b][p]paletteuse=dither=bayer',
                    '-loop','0',str(assets/'agent-budget.gif')],check=True)
    intro.save(assets/'agent-budget-poster.png')
    sheet=Image.new('RGB',(W,400*((len(thumbs)+1)//2)),BG)
    for n,im in enumerate(thumbs): sheet.paste(im.resize((640,400)),((n%2)*640,(n//2)*400))
    sheet.save('/tmp/zwallet-launch-contact-sheet.png')
    print(json.dumps({'durationSeconds':round(6+duration+1+10+10+7,2),
                      'reviewSnapshots':len(thumbs),'mp4Bytes':(assets/'agent-budget.mp4').stat().st_size,
                      'gifBytes':(assets/'agent-budget.gif').stat().st_size}))

if __name__=='__main__': main()
