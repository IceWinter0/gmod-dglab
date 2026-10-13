export class DeveloperAccess {
  enabled=false;
  clicks=0;
  private lastClick:number|null=null;
  click(now:number) {
    if(this.enabled)return;
    if(this.lastClick===null || now-this.lastClick>3000 || now<this.lastClick)this.clicks=0;
    this.lastClick=now;
    this.clicks++;
    if(this.clicks===7)this.enabled=true;
  }
  exit() {this.enabled=false;this.clicks=0;this.lastClick=null;}
}
