#!/usr/bin/env python3
"""Original monochrome motion explainer, with English and Korean editions.

Outcome-first story; no terminal or simulated product dashboard. Tone follows the
owner's icedac monochrome profile: sparse type, fine rules, negative space and slow
geometric motion. Request/return flow is inspired by the supplied agent-graph example.
"""
import argparse
from functools import lru_cache
import json
import math
from pathlib import Path
import subprocess

import imageio_ffmpeg
from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parents[2]
W, H, FPS = 1600, 900, 60
SCALE = 2
DURATIONS = [7, 8, 10, 14, 12, 7]
DURATION = sum(DURATIONS)
BG, INK, SUB, RULE = '#080808', '#F5F5F5', '#A3A3A3', '#303030'
COPY = {
 'en': {
  'title':['AI can build.\nYou still pay.', 'A working\nallowance.', 'A task done.\nA clear account.',
           'Your money.\nYour rules.', 'What happens\nnext.', 'Give your agent\na wallet.'],
  'body':['It writes the code.\nThen asks you to send funds,\npay, and check the result.',
          'Fund a separate wallet.\nChoose where, how much per payment,\nand how long it can act.',
          'Your agent handles the payment,\nchecks the result, and reports back.',
          'Allowed destinations.\nLimits for each payment.\nAccess you can end.',
          'Development today.\nPayments, purchases and investment\nthrough future integrations.',
          'Your agent. Your money. Your rules.'],
  'you':'You', 'agent':'Your agent', 'app':'Your app', 'wallet':'zwallet', 'device':'YOUR DEVICE',
  'keys':'Keys stay here', 'funds':'TASK FUNDS', 'request':'PAYMENT REQUEST', 'access':'30 MIN ACCESS',
  'asks':['SEND FUNDS','MAKE PAYMENT','VERIFY RESULT'], 'working':'WORKING',
  'done':'COMPLETE', 'confirmed':'Payment verified.', 'report':'Amount / fees / remaining funds',
  'summary':'AGENT REPORT', 'allowed':'Allowed', 'blocked':'Blocked', 'payee':'Approved payee',
  'unknown':'Not on your list', 'lock_title':'Until you\nsay stop.', 'locked':'ACCESS CLOSED',
  'build':'Development', 'pay':'Service payments', 'buy':'Purchases', 'invest':'Investment plan',
  'open':'Open source. Local keys. No service signup.',
  'header':'A WALLET FOR YOUR AGENT', 'future':'FUTURE / WIP',
  'note':'Illustrated workflow', 'boundary':'Per-payment limits are not cumulative budgets. Use a trusted device.',
  'risk':'Future integrations. Investment outcomes can include losses.',
  'chapters':['THE PROBLEM','THE ALLOCATION','THE RESULT','THE CONTROL','THE POSSIBILITY','YOUR NEXT STEP'],
 },
 'ko': {
  'title':['AI가 만들어도,\n결제는 아직 내 몫.', '작업비를\n맡기세요.', '일은 끝내고,\n내역은 남기고.',
           '내 돈.\n내가 정한 규칙.', '그다음의\n가능성.', '내 에이전트에게\n지갑을 주세요.'],
  'body':['코드는 알아서 짜면서,\n돈 보내고 결제하고 확인하는 일은\n계속 나를 부릅니다.',
          '작업용 지갑에 자금을 넣고,\n어디에, 한 번에 얼마까지,\n언제까지 쓸지 내가 정합니다.',
          '에이전트가 결제를 처리하고,\n결과와 사용 내역을 가져옵니다.',
          '내가 허용한 수취인.\n거래마다 정해둔 한도.\n언제든 종료할 수 있는 접근.',
          '오늘은 개발에.\n앞으로는 서비스 결제와 물건 구매,\n내가 정한 투자 계획까지.',
          '내 에이전트. 내 돈. 내가 정한 규칙.'],
  'you':'나', 'agent':'내 에이전트', 'app':'내 앱', 'wallet':'zwallet', 'device':'내 기기',
  'keys':'키는 여기에', 'funds':'맡긴 작업비', 'request':'결제 요청', 'access':'30분 동안 접근',
  'asks':['자금 보내주세요','결제해주세요','결과 확인해주세요'], 'working':'작업 중',
  'done':'완료', 'confirmed':'결제 확인 완료.', 'report':'사용액 / 수수료 / 남은 금액',
  'summary':'에이전트의 보고', 'allowed':'허용', 'blocked':'거절', 'payee':'허용한 수취인',
  'unknown':'허용하지 않은 수취인', 'lock_title':'내가 멈추면,\n접근도 끝.', 'locked':'접근 종료',
  'build':'개발', 'pay':'서비스 결제', 'buy':'물건 구매', 'invest':'내 투자 계획',
  'open':'오픈소스. 로컬 키. 별도 서비스 가입 없음.',
  'header':'내 에이전트를 위한 지갑', 'future':'미래 기능 / WIP',
  'note':'작동 원리를 보여주는 설명 영상',
  'boundary':'거래별 한도는 누적 예산이 아닙니다. 신뢰하는 기기에서 사용하세요.',
  'risk':'미래 연동 기능입니다. 투자에는 손실이 발생할 수 있습니다.',
  'chapters':['떠넘겨진 일','작업비','결과','통제','가능성','시작'],
 }
}


def ease(v):
    v = min(1, max(0, v))
    return v*v*(3-2*v)


def rgba(color, opacity=1):
    return tuple(int(color[i:i+2],16) for i in (1,3,5))+(round(255*max(0,min(1,opacity))),)


@lru_cache(maxsize=100)
def font(size,lang='en',meta=False):
    if meta and lang=='en':
        choices=[('/System/Library/Fonts/Menlo.ttc',0),('/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf',0)]
    elif lang=='ko':
        choices=[('/System/Library/Fonts/AppleSDGothicNeo.ttc',2),('/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc',1)]
    else:
        choices=[('/System/Library/Fonts/Avenir Next.ttc',5),('/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf',0)]
    for path,index in choices:
        if Path(path).exists(): return ImageFont.truetype(path,size*SCALE,index=index)
    raise RuntimeError('Install the documented font for '+lang)


def text(d,xy,s,size=25,color=INK,lang='en',anchor=None,a=1,meta=False,maxwidth=None):
    f=font(size,lang,meta)
    while maxwidth and d.textbbox((0,0),s,font=f)[2]>maxwidth and size>17:
        size-=1; f=font(size,lang,meta)
    d.text(xy,s,font=f,fill=rgba(color,a),anchor=anchor)


def rule(d,start,end,a=1,width=1,color=RULE):
    d.line([start,end],fill=rgba(color,a),width=width)


class Drawing:
    """Draw in logical coordinates at 2x resolution for stable subpixel motion."""
    def __init__(self, image): self.d=ImageDraw.Draw(image,'RGBA')
    @staticmethod
    def coords(xy):
        if xy and isinstance(xy[0],(tuple,list)):
            return [(x*SCALE,y*SCALE) for x,y in xy]
        return tuple(v*SCALE for v in xy)
    def text(self,xy,s,**kw): self.d.text(self.coords(xy),s,**kw)
    def textbbox(self,xy,s,**kw):
        return tuple(v/SCALE for v in self.d.textbbox(self.coords(xy),s,**kw))
    def line(self,xy,**kw):
        kw['width']=round(kw.get('width',1)*SCALE)
        self.d.line(self.coords(xy),**kw)
    def ellipse(self,xy,**kw):
        kw['width']=round(kw.get('width',1)*SCALE)
        self.d.ellipse(self.coords(xy),**kw)
    def rectangle(self,xy,**kw):
        kw['width']=round(kw.get('width',1)*SCALE)
        self.d.rectangle(self.coords(xy),**kw)
    def arc(self,xy,start,end,**kw):
        kw['width']=round(kw.get('width',1)*SCALE)
        self.d.arc(self.coords(xy),start,end,**kw)


@lru_cache(maxsize=8)
def field_template(r,family):
    rings=[]
    for j in range(36):
        angle=j*math.pi/36
        rx=r*(.87+.09*math.sin(j*.26+family))
        ry=r*(.31+.07*math.cos(j*.19+family))
        ca,sa=math.cos(angle),math.sin(angle)
        points=[]
        for k in range(129):
            theta=k*math.tau/128; u,v=rx*math.cos(theta),ry*math.sin(theta)
            points.append((u*ca-v*sa,u*sa+v*ca))
        rings.append(points)
    return rings


def linefield(d,x,y,r,t,a=1):
    """Original nested elliptical field; a slow, geometric wallet focal point."""
    for family in (0,1):
        phase=.09*math.sin(t/(9+family*3))+family*.06
        ca,sa=math.cos(phase),math.sin(phase)
        for ring in field_template(r,family):
            points=[(x+u*ca-v*sa,y+.92*(u*sa+v*ca)) for u,v in ring]
            d.line(points,fill=rgba(INK,a*(.37 if family==0 else .13)),width=1)


def mark(d,kind,x,y,r=28,a=1):
    col=rgba(INK,a)
    if kind=='user':
        d.ellipse((x-r,y-r,x+r,y+r),outline=col,width=1)
        d.ellipse((x-4,y-4,x+4,y+4),fill=col)
    elif kind=='agent':
        d.ellipse((x-r,y-r,x+r,y+r),outline=col,width=1)
        for k in range(4):
            angle=k*math.pi/2
            xx,yy=x+12*math.cos(angle),y+12*math.sin(angle)
            d.ellipse((xx-2,yy-2,xx+2,yy+2),fill=col)
    elif kind in ('app','service','shop'):
        d.rectangle((x-r,y-r,x+r,y+r),outline=col,width=1)
        if kind=='app':
            rule(d,(x-r,y-r+13),(x+r,y-r+13),a,color=INK)
        elif kind=='service':
            for off in (-9,9): rule(d,(x-14,y+off),(x+14,y+off),a,color=INK)
        else:
            d.arc((x-11,y-16,x+11,y+7),180,360,fill=col,width=1)
            rule(d,(x-11,y-5),(x-11,y+4),a,color=INK)
            rule(d,(x+11,y-5),(x+11,y+4),a,color=INK)
    elif kind=='check':
        d.line([(x-17,y),(x-4,y+12),(x+20,y-14)],fill=col,width=2)
    elif kind=='block':
        rule(d,(x-14,y-14),(x+14,y+14),a,2,INK)
        rule(d,(x-14,y+14),(x+14,y-14),a,2,INK)
    elif kind=='invest':
        rule(d,(x-r,y-r),(x-r,y+r),a,color=INK)
        rule(d,(x-r,y+r),(x+r,y+r),a,color=INK)
        d.line([(x-19,y+6),(x-5,y-11),(x+9,y+2),(x+23,y-18)],fill=col,width=1)
    elif kind=='lock':
        d.rectangle((x-19,y-1,x+19,y+25),outline=col,width=1)
        d.arc((x-12,y-24,x+12,y+8),180,360,fill=col,width=1)
        rule(d,(x-12,y-8),(x-12,y),a,color=INK)
        rule(d,(x+12,y-8),(x+12,y),a,color=INK)


def node(d,kind,x,y,label,lang,t,a=1,r=28,detail=None):
    mark(d,kind,x,y,r,a)
    text(d,(x,y+r+30),label,23,lang=lang,anchor='mm',a=a,maxwidth=280)
    if detail: text(d,(x,y+r+62),detail,17,SUB,lang,anchor='mm',a=a,meta=True,maxwidth=280)


def wallet(d,x,y,lang,t,r=95,locked=False):
    linefield(d,x,y,r,10.6 if locked else t)
    if locked:
        d.rectangle((x-30,y-30,x+30,y+32),fill=rgba(BG))
        mark(d,'lock',x,y,20)
    text(d,(x,y+r+30),'zwallet',26,lang='en',anchor='mm')


def edge(d,start,end,t,delay=0,bend=0,stop=1,flow=True,a=1,ghost=False):
    x0,y0=start; x1,y1=end; pts=[]
    for k in range(65):
        u=k/64
        pts.append((x0+(x1-x0)*u,y0+(y1-y0)*u+math.sin(u*math.pi)*bend))
    if ghost:
        for i in range(0,62,5): d.line(pts[i:i+3],fill=rgba(SUB,.45*a),width=1)
    else: d.line(pts,fill=rgba(SUB,.38*a),width=1)
    if not flow or t<delay: return
    u=((t-delay)*.23)%1
    if u>stop: return
    # Evaluate the curve continuously; indexing a point list caused visible steps.
    x,y=x0+(x1-x0)*u,y0+(y1-y0)*u+math.sin(u*math.pi)*bend
    opacity=a*ease(u/.05)*ease((stop-u)/.05)
    if ghost: d.ellipse((x-4,y-4,x+4,y+4),outline=rgba(INK,opacity),width=1)
    else: d.ellipse((x-3,y-3,x+3,y+3),fill=rgba(INK,opacity))


def scene_frame(i,t,lang,global_t):
    q=COPY[lang]; im=Image.new('RGB',(W*SCALE,H*SCALE),BG); d=Drawing(im)
    text(d,(80,53),'zwallet.',31)
    text(d,(1520,65),q['future'] if i==4 else q['header'],17,SUB,lang,anchor='rm',meta=True)
    closed=i==3 and t>=10.6
    title=q['lock_title'] if closed else q['title'][i]
    yy=233+12*(1-ease(t/.9))
    for n,s in enumerate(title.split('\n')):
        text(d,(74,yy+n*104),s,91 if lang=='en' else 78,lang=lang,a=.45+.55*ease(t/.8),maxwidth=680)
    for n,s in enumerate(q['body'][i].split('\n')):
        text(d,(80,490+n*44),s,28,SUB,lang,maxwidth=625)
    note=q['boundary'] if i==3 else q['risk'] if i==4 else q['note']
    text(d,(80,775),note,17,SUB,lang,maxwidth=1430)
    rule(d,(80,820),(1520,820))
    rule(d,(80,820),(80+1440*global_t/DURATION,820),color=INK)
    text(d,(80,851),f'{i+1:02d} / '+q['chapters'][i],17,SUB,lang,meta=True)
    text(d,(1520,851),'github.com/2lab-ai/zwallet',18,SUB,anchor='ra',meta=True)

    if i==0:
        edge(d,(1072,370),(1432,370),t)
        edge(d,(1084,394),(850,535),t,delay=.7,bend=18)
        node(d,'agent',1100,370,q['agent'],lang,t)
        node(d,'app',1460,370,q['app'],lang,t)
        node(d,'user',830,555,q['you'],lang,t)
        for j,s in enumerate(q['asks']):
            a=ease((t-.8-j*1.2)/.9); y=510+j*61
            rule(d,(990,y),(1470,y),a)
            text(d,(1000,y+19),s,18,lang=lang,a=a,meta=True)
    elif i==1:
        d.rectangle((969,326,1271,626),outline=rgba(RULE),width=1)
        text(d,(1120,300),q['device'],17,SUB,lang,anchor='mm',meta=True)
        edge(d,(838,470),(1025,470),t,delay=.6)
        edge(d,(1422,470),(1215,470),t,delay=2.8)
        node(d,'user',810,470,q['you'],lang,t)
        node(d,'agent',1450,470,q['agent'],lang,t)
        wallet(d,1120,470,lang,t)
        text(d,(925,421),q['funds'],16,SUB,lang,anchor='mm',a=ease(t-1),meta=True)
        text(d,(1330,421),q['request'],16,SUB,lang,anchor='mm',a=ease(t-3),meta=True)
        text(d,(1120,661),q['keys']+' / '+q['access'],18,SUB,lang,anchor='mm',maxwidth=640)
    elif i==2:
        edge(d,(840,420),(1020,420),t,delay=.2)
        edge(d,(1210,420),(1422,420),t,delay=1.7)
        edge(d,(1422,449),(1210,449),t,delay=3.5,bend=8)
        node(d,'agent',812,420,q['agent'],lang,t)
        node(d,'app',1450,420,q['app'],lang,t)
        wallet(d,1115,420,lang,t)
        a=ease((t-5.2)/.9)
        if t<5.2: text(d,(1115,643),q['working'],17,SUB,lang,anchor='mm',meta=True,a=1-ease((t-4.5)/.7))
        else:
            rule(d,(815,600),(1500,600),a)
            text(d,(830,622),q['summary']+' / '+q['done'],16,SUB,lang,a=a,meta=True)
            mark(d,'check',845,681,18,a)
            text(d,(880,655),q['confirmed'],36,lang=lang,a=a,maxwidth=590)
            text(d,(880,707),q['report'],22,SUB,lang,a=a,maxwidth=590)
    elif i==3:
        edge(d,(842,460),(1020,460),t,flow=not closed)
        edge(d,(1205,428),(1420,327),t,flow=not closed)
        edge(d,(1205,494),(1420,600),t,delay=3,stop=.17,flow=not closed)
        node(d,'user' if closed else 'agent',814,460,q['you'] if closed else q['agent'],lang,t)
        wallet(d,1115,460,lang,t,locked=closed)
        node(d,'check',1450,315,q['payee'],lang,t,r=23)
        node(d,'block',1450,614,q['unknown'],lang,t,r=23,a=ease(t-2.4))
        text(d,(1315,345),q['allowed'],17,SUB,lang,anchor='mm',a=ease(t-1),meta=True)
        text(d,(1310,596),q['blocked'],17,INK,lang,anchor='mm',a=ease(t-3),meta=True)
        if t>=3 and not closed: mark(d,'block',1244,513,12,ease(t-3))
        text(d,(1115,655),q['locked'] if closed else q['keys'],18,SUB,lang,anchor='mm',meta=closed)
    elif i==4:
        points=[(805,500,'build','app'),(1390,285,'pay','service'),(1450,500,'buy','shop'),(1195,690,'invest','invest')]
        for j,(x,y,label,kind) in enumerate(points):
            a=ease((t-.4-j*.65)/1.1)
            edge(d,(1115,455),(x,y),t,delay=.5+j*.65,ghost=j>0,a=a)
            node(d,kind,x,y,q[label],lang,t,r=23,a=a)
            if j: text(d,(x,y-48),'WIP',16,SUB,anchor='mm',meta=True,a=a)
        wallet(d,1115,455,lang,t)
    else:
        linefield(d,1160,448,265,t*.7)
        text(d,(80,610),q['open'],22,SUB,lang,maxwidth=660)
    return im.resize((W,H),Image.Resampling.LANCZOS)


def frame_at(t,lang):
    start=0
    for i,duration in enumerate(DURATIONS):
        if t<start+duration:
            local=t-start; current=scene_frame(i,local,lang,t)
            if i and local<1.0:
                return Image.blend(scene_frame(i-1,DURATIONS[i-1]-.001,lang,t),current,ease(local))
            if i==3 and 10.6<=local<11.6:
                return Image.blend(scene_frame(i,10.599,lang,t),current,ease(local-10.6))
            return current
        start+=duration
    return scene_frame(5,DURATIONS[-1]-.001,lang,DURATION)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--lang',choices=['en','ko'],default='en')
    parser.add_argument('--output',type=Path,default=ROOT/'docs/assets')
    parser.add_argument('--stills-only',action='store_true')
    args=parser.parse_args(); args.output.mkdir(parents=True,exist_ok=True)
    name='agent-story'+('-ko' if args.lang=='ko' else '')
    moments=[3.8,12.4,23.6,29.8,35,38,44.8,55]
    thumbs=[]
    for index,t in enumerate(moments):
        im=frame_at(t,args.lang); thumbs.append(im)
        im.save(f'/tmp/zwallet-story-{args.lang}-{index+1}.png')
    sheet=Image.new('RGB',(1600,1800),BG)
    for i,im in enumerate(thumbs): sheet.paste(im.resize((800,450)),((i%2)*800,(i//2)*450))
    sheet.save(f'/tmp/zwallet-story-{args.lang}-contact.png')
    frame_at(54,args.lang).save(args.output/(name+'-poster.png'))
    if args.stills_only: return
    ff=imageio_ffmpeg.get_ffmpeg_exe(); video=args.output/(name+'.mp4')
    p=subprocess.Popen([ff,'-y','-loglevel','error','-f','rawvideo','-vcodec','rawvideo','-pix_fmt','rgb24',
                        '-s',f'{W}x{H}','-r',str(FPS),'-i','-','-an','-c:v','libx264','-preset','medium',
                        '-crf','20','-pix_fmt','yuv420p','-movflags','+faststart',str(video)],stdin=subprocess.PIPE)
    for n in range(DURATION*FPS): p.stdin.write(frame_at(n/FPS,args.lang).tobytes())
    p.stdin.close()
    if p.wait(): raise RuntimeError('Video encoding failed')
    print(json.dumps({'language':args.lang,'seconds':DURATION,'fps':FPS,'size':[W,H],
                      'style':'monochrome','videoBytes':video.stat().st_size,'reviewSnapshots':len(thumbs)}))


if __name__=='__main__': main()
