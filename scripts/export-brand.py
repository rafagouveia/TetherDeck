from pathlib import Path
from io import BytesIO
import cairosvg
from PIL import Image, ImageDraw, ImageFont

brand = Path('assets/brand')
app = (brand/'tetherdeck-app.svg').read_bytes()
mark = (brand/'tetherdeck-mark.svg').read_bytes()
for size in (64, 256):
    cairosvg.svg2png(bytestring=app, write_to=str(brand/f'tetherdeck-icon-{size}.png'), output_width=size, output_height=size)
cairosvg.svg2png(bytestring=mark, write_to=str(brand/'tetherdeck-symbol.png'), output_width=256, output_height=256)
icon = Image.open(BytesIO(cairosvg.svg2png(bytestring=app, output_width=256, output_height=256)))
icon.save(brand/'tetherdeck.ico', format='ICO', sizes=[(16,16),(24,24),(32,32),(48,48),(64,64),(128,128),(256,256)])
# Repository banner derives from the same vector identity and theme tokens.
banner = Image.new('RGB', (1280, 400), '#21252B')
banner.paste(icon, (100,72), icon)
draw = ImageDraw.Draw(banner)
font = ImageFont.truetype('C:/Windows/Fonts/seguisb.ttf', 76)
small = ImageFont.truetype('C:/Windows/Fonts/segoeui.ttf', 25)
draw.text((410,116), 'TetherDeck', font=font, fill='#DCE0E5')
draw.text((413,223), 'Connect. Inspect. Control.', font=small, fill='#A9AFBC')
banner.save(brand/'tetherdeck-banner.png')
