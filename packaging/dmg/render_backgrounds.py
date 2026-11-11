"""Development-only deterministic artwork export. Requires Pillow, not used at build time."""
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont
BASE=Path(__file__).resolve().parent
FONT='/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf'
BOLD='/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf'
APPS={'solmu': ('Solmu', '#54dab9')}
def draw(name,color):
 im=Image.new('RGB',(4096,2160),'#171a20');d=ImageDraw.Draw(im)
 d.rounded_rectangle((30,30,36,108),3,fill=color)
 d.text((54,29),'Welcome to '+name,font=ImageFont.truetype(BOLD,31),fill='#f6f5f0')
 d.text((55,80),'Drag the app to Applications to install.',font=ImageFont.truetype(FONT,17),fill='#bdc4ce')
 for x in [96,473]:d.rounded_rectangle((x,166,x+206,373),24,fill='#eef0ed')
 d.text((388,221),'→',font=ImageFont.truetype(FONT,54),fill='#bfc8d2',anchor='mt')
 d.text((55,396),'Open from Applications when the copy is finished.',font=ImageFont.truetype(FONT,16),fill='#bdc4ce')
 return im
if __name__=='__main__':
 for slug,(name,color) in APPS.items():draw(name,color).save(BASE/('welcome-'+slug+'.png'),dpi=(72,72))
