import megapi
from enum import Enum

class DcMotor(Enum):
    FrontRight = megapi.A1
    FrontLeft = megapi.A2
    BackRight = megapi.A3
    BackLeft = megapi.A4
