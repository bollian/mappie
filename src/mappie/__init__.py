from megapi import MegaPi
from mappie.ports import DcMotor
from time import sleep


def main():
    print("Hello, World!")
    bot = MegaPi()
    bot.start()

    for p in range(8):
        print(f'running motor {p}')
        bot.motorRun(p, 1.0)
        sleep(1)
