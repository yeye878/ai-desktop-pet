let storage = new LocalStorage();

@Entry(storage)
@Component
struct PetCard {
  @LocalStorageProp('status') status: string = '我在等你哦～';
  @LocalStorageProp('emoji') emoji: string = '🐹';

  build() {
    Column({ space: 6 }) {
      Row({ space: 8 }) {
        Text(this.emoji).fontSize(30)
        Column({ space: 2 }) {
          Text('智能伙伴')
            .fontSize(13)
            .fontWeight(FontWeight.Bold)
            .fontColor('#5B4A4F')
          Text(this.status)
            .fontSize(11)
            .fontColor('#8A7A80')
            .maxLines(2)
        }
        .alignItems(HorizontalAlign.Start)
        .layoutWeight(1)
      }
      .width('100%')

      Blank()

      Button('💬 和我聊天')
        .width('100%')
        .height(32)
        .fontSize(13)
        .backgroundColor('#F76C8A')
        .onClick(() => {
          postCardAction(this, {
            'action': 'router',
            'abilityName': 'EntryAbility',
            'params': { 'page': 'chat' }
          });
        })
    }
    .width('100%')
    .height('100%')
    .padding(12)
    .backgroundColor('#FDF3F0')
    .onClick(() => {
      postCardAction(this, {
        'action': 'router',
        'abilityName': 'EntryAbility',
        'params': { 'page': 'home' }
      });
    })
  }
}
