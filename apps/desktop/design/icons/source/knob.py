import math
G=lambda glow:f'''<linearGradient id="bg" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#3A3A40"/><stop offset="1" stop-color="#161618"/></linearGradient>
<radialGradient id="kn" cx=".5" cy=".3" r=".75"><stop offset="0" stop-color="#55555D"/><stop offset="1" stop-color="#1F1F23"/></radialGradient>
<linearGradient id="arc" x1="0" y1="1" x2="1" y2="0"><stop offset="0" stop-color="#4C7DF2"/><stop offset="1" stop-color="#93B3FF"/></linearGradient>
<filter id="gl" x="-30%" y="-30%" width="160%" height="160%"><feGaussianBlur stdDeviation="{glow}"/></filter>
<filter id="sh" x="-20%" y="-20%" width="140%" height="150%"><feGaussianBlur in="SourceAlpha" stdDeviation="14"/><feOffset dy="10"/><feComponentTransfer><feFuncA type="linear" slope=".35"/></feComponentTransfer><feMerge><feMergeNode/><feMergeNode in="SourceGraphic"/></feMerge></filter>'''
def svg(body,glow=22,vb=1024):
    return f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {vb} {vb}"><defs>{G(glow)}</defs>{body}</svg>'
def pt(c,r,phi): return (c[0]+r*math.sin(math.radians(phi)), c[1]-r*math.cos(math.radians(phi)))
def arc(c,r,a0,a1):
    x0,y0=pt(c,r,a0);x1,y1=pt(c,r,a1); large=1 if (a1-a0)>180 else 0
    return f'M{x0:.2f} {y0:.2f} A{r:.2f} {r:.2f} 0 {large} 1 {x1:.2f} {y1:.2f}'
V=45
def knob(c=(512,522),s=1.0,mode='color',ink='#FFFFFF',parts=('arc','knob')):
    small=mode=='small'; mono=mode=='mono'
    R=278*s; sw=(56 if small else 36)*s; o=''
    kr=(205 if not small else 190)*s
    if 'arc' in parts:
        o+=f'<path d="{arc(c,R,-135,135)}" fill="none" stroke="{ink if mono else "#fff"}" stroke-opacity="{.3 if mono else .09}" stroke-width="{sw:.1f}" stroke-linecap="round"/>'
        if mode=='color': o+=f'<path d="{arc(c,R,-135,V)}" fill="none" stroke="#3A6AE0" stroke-opacity=".8" stroke-width="{sw:.1f}" stroke-linecap="round" filter="url(#gl)"/>'
        o+=f'<path d="{arc(c,R,-135,V)}" fill="none" stroke="{ink if mono else "url(#arc)"}" stroke-width="{sw:.1f}" stroke-linecap="round"/>'
    if 'knob' in parts:
        if mono:
            o+=f'<circle cx="{c[0]}" cy="{c[1]}" r="{kr-14*s:.1f}" fill="{ink}" fill-opacity=".22" stroke="{ink}" stroke-width="{22*s:.1f}"/>'
        elif small:
            o+=f'<circle cx="{c[0]}" cy="{c[1]}" r="{kr:.1f}" fill="url(#kn)"/>'
        else:
            o+=f'<circle cx="{c[0]}" cy="{c[1]+10*s:.1f}" r="{kr:.1f}" fill="#000" opacity=".45" filter="url(#gl)"/>'
            o+=f'<circle cx="{c[0]}" cy="{c[1]}" r="{kr:.1f}" fill="#18181B"/>'
            o+=f'<circle cx="{c[0]}" cy="{c[1]}" r="{kr-9*s:.1f}" fill="none" stroke="#4A4A51" stroke-width="{16*s:.1f}" stroke-dasharray="{9*s:.2f} {9.6*s:.2f}"/>'
            o+=f'<circle cx="{c[0]}" cy="{c[1]}" r="{kr-26*s:.1f}" fill="url(#kn)"/>'
            o+=f'<circle cx="{c[0]}" cy="{c[1]}" r="{kr-26*s:.1f}" fill="none" stroke="#fff" stroke-opacity=".12" stroke-width="{3*s:.1f}"/>'
        a=pt(c,(40 if small else 60)*s,V); b=pt(c,kr-(40 if small else 52)*s,V)
        o+=f'<path d="M{a[0]:.1f} {a[1]:.1f} L{b[0]:.1f} {b[1]:.1f}" stroke="{ink if mono else "#EEF0F5"}" stroke-width="{(54 if small else 30)*s:.1f}" stroke-linecap="round"/>'
    return o
EDGE='<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{r}" fill="none" stroke="#fff" stroke-opacity=".10" stroke-width="3"/>'
MAC='<rect x="100" y="100" width="824" height="824" rx="185" fill="url(#bg)" filter="url(#sh)"/>'+EDGE.format(x=101.5,y=101.5,w=821,h=821,r=183.5)
MAC_NOSH='<rect x="100" y="100" width="824" height="824" rx="185" fill="url(#bg)"/>'+EDGE.format(x=101.5,y=101.5,w=821,h=821,r=183.5)
FULL='<rect width="1024" height="1024" fill="url(#bg)"/>'
BLACK='<rect width="1024" height="1024" fill="#000"/>'
WIN='<rect x="64" y="64" width="896" height="896" rx="128" fill="url(#bg)"/>'+EDGE.format(x=66,y=66,w=892,h=892,r=126)
WIN_S='<rect x="32" y="32" width="960" height="960" rx="150" fill="url(#bg)"/>'
GN='<rect x="96" y="150" width="832" height="784" rx="150" fill="#0B0B0D"/><rect x="96" y="112" width="832" height="784" rx="150" fill="url(#bg)"/>'+EDGE.format(x=98,y=114,w=828,h=780,r=148)
GN_S='<rect x="64" y="96" width="896" height="864" rx="150" fill="#0B0B0D"/><rect x="64" y="64" width="896" height="864" rx="150" fill="url(#bg)"/>'
FAV='<rect x="32" y="32" width="960" height="960" rx="220" fill="url(#bg)"/>'
def symbolic(color):
    c=(8,8.6); R=6.6
    a=pt(c,0.6,V); b=pt(c,2.6,V)
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><defs><mask id="m"><rect width="16" height="16" fill="#fff"/>'
      f'<path d="M{a[0]:.2f} {a[1]:.2f} L{b[0]:.2f} {b[1]:.2f}" stroke="#000" stroke-width="1.3" stroke-linecap="round"/></mask></defs>'
      f'<path d="{arc(c,R,-135,135)}" fill="none" stroke="{color}" stroke-opacity=".35" stroke-width="1.5" stroke-linecap="round"/>'
      f'<path d="{arc(c,R,-135,V)}" fill="none" stroke="{color}" stroke-width="1.5" stroke-linecap="round"/>'
      f'<circle cx="8" cy="8.6" r="3.6" fill="{color}" mask="url(#m)"/></svg>')
FILES={
 'macos.svg':svg(MAC+knob()),
 'macos-small.svg':svg(MAC+knob(s=1.05,mode='small')),
 'macos-tile.svg':svg(MAC_NOSH+knob()),
 'full.svg':svg(FULL+knob(c=(512,522),s=1.2),glow=26),
 'full-small.svg':svg(FULL+knob(c=(512,522),s=1.25,mode='small')),
 'ios-dark.svg':svg(BLACK+knob(c=(512,522),s=1.2),glow=26),
 'ios-tinted.svg':svg(BLACK+knob(c=(512,522),s=1.2,mode='mono')),
 'mark.svg':svg(knob()),
 'mark-android.svg':svg(knob(c=(512,520),s=0.95),glow=21),
 'mark-mono.svg':svg(knob(mode='mono')),
 'mark-mono-android.svg':svg(knob(c=(512,520),s=0.95,mode='mono')),
 'bg.svg':svg(FULL),
 'layer-arc.svg':svg(knob(parts=('arc',))),
 'layer-knob.svg':svg(knob(parts=('knob',))),
 'windows.svg':svg(WIN+knob(c=(512,522),s=1.1),glow=24),
 'windows-small.svg':svg(WIN_S+knob(c=(512,522),s=1.17,mode='small')),
 'gnome.svg':svg(GN+knob(c=(512,512),s=1.0)),
 'gnome-small.svg':svg(GN_S+knob(c=(512,502),s=1.1,mode='small')),
 'symbolic-dark.svg':symbolic('#2E3436'),
 'symbolic-light.svg':symbolic('#FFFFFF'),
 'favicon.svg':svg(FAV+knob(c=(512,522),s=1.17,mode='small')),
 'maskable.svg':svg(FULL+knob(c=(512,520),s=0.95),glow=21),
}
if __name__=='__main__':
    for k,v in FILES.items(): open('outb/'+k,'w').write(v)
